//! Per-input-context session state machine.
//!
//! Two modes:
//!   - **Composing**: the user types romaji, shown as hiragana (see [`romaji`]).
//!     An *auto-convert* (typing-pause) conversion populates `candidates` while
//!     **staying in Composing** — a non-committal **preview**: the preedit stays
//!     the raw romaji and **Enter commits it as-typed**. The user **Space**s to
//!     *engage* the preview (→ Candidates) or keeps typing to dismiss it.
//!   - **Candidates**: reached by an *explicit* Space conversion, or by Space-ing
//!     an auto-convert preview. A ranked candidate list is shown; Space/arrows
//!     cycle, Enter / number keys commit, Escape cancels back to the romaji.
//!
//! This split is the whole answer to "sometimes I want Enter to convert, sometimes
//! not": conversion only becomes *committable by Enter* once **you** press Space.
//! It never depends on whether the auto-convert pause happened to fire.
//!
//! Cloud-AI conversion is asynchronous: [`Session::begin_ai_convert`] spawns a
//! background thread for the (slow) LLM call and returns a request id;
//! [`Session::poll_ai_result`] is called repeatedly on the *same* thread as the
//! other session calls until it reports [`AiPoll::Ready`]/[`AiPoll::Error`]. The
//! only thing that touches another thread is the HTTP call itself, which writes
//! into a shared slot — the `Session` is never accessed concurrently.

use crate::ai::{AiPoll, ConvertRequest, Converter};
use crate::key::{flags, keysym, Key};
use crate::learning::Learning;
use crate::romaji;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Composing,
    Candidates,
}

/// CANDIDATES flag iff an auto-convert preview was just torn down, so the
/// frontend hides its suggestion window.
const fn preview_flag(had_preview: bool) -> u32 {
    if had_preview {
        flags::CANDIDATES
    } else {
        0
    }
}

/// Shared state of an in-flight (possibly streaming) AI request, updated by the
/// background thread: `candidates` grows as the model streams, `done` flips when
/// finished, `error` is set on failure.
#[derive(Default)]
struct Slot {
    candidates: Vec<String>,
    done: bool,
    error: Option<String>,
}

/// State for a single input context.
pub struct Session {
    raw: String,
    preedit: String,
    candidates: Vec<String>,
    highlighted: usize,
    commit: String,
    mode: Mode,

    converter: Option<Arc<dyn Converter>>,
    next_req: u64,
    slots: HashMap<u64, Arc<Mutex<Slot>>>,
    /// Message from the most recent failed AI conversion (for diagnostics).
    last_error: String,
    /// Shared usage learning (promotes previously-chosen candidates).
    learning: Arc<Mutex<Learning>>,
    /// Whether the in-flight conversion was triggered explicitly (Space) — and so
    /// should enter Candidates mode on completion — vs. by the auto-convert pause,
    /// which only shows a preview (stays in Composing). Set by `begin_ai_convert`.
    convert_explicit: bool,
}

impl Session {
    pub(crate) fn new(
        converter: Option<Arc<dyn Converter>>,
        learning: Arc<Mutex<Learning>>,
    ) -> Self {
        Session {
            raw: String::new(),
            preedit: String::new(),
            candidates: Vec::new(),
            highlighted: 0,
            commit: String::new(),
            mode: Mode::Composing,
            converter,
            next_req: 1,
            slots: HashMap::new(),
            last_error: String::new(),
            learning,
            convert_explicit: false,
        }
    }

    // --- accessors -------------------------------------------------------

    pub fn preedit(&self) -> &str {
        &self.preedit
    }
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }
    pub fn commit_text(&self) -> &str {
        &self.commit
    }
    /// Message from the most recent failed AI conversion (empty if none).
    pub fn last_error(&self) -> &str {
        &self.last_error
    }
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty() && self.candidates.is_empty()
    }
    /// The best-effort local kana for the current romaji (used as AI context /
    /// fallback). Empty unless composing.
    pub fn current_kana(&self) -> String {
        romaji::flush(&self.raw)
    }

    // --- internal helpers ------------------------------------------------

    fn refresh_preedit(&mut self) {
        self.preedit = match self.mode {
            Mode::Candidates => self
                .candidates
                .get(self.highlighted)
                .cloned()
                .unwrap_or_default(),
            // AI mode (no mode switching): show the raw romaji the user typed so
            // English/identifiers look natural; the AI converts the whole buffer
            // (JP + English) on a pause or on Space/Enter. Offline (no converter),
            // fall back to showing the romaji->kana transliteration.
            Mode::Composing if self.converter.is_some() => self.raw.clone(),
            Mode::Composing => {
                let (kana, pending) = romaji::convert(&self.raw);
                if pending == "n" {
                    format!("{kana}ん")
                } else {
                    format!("{kana}{pending}")
                }
            }
        };
    }

    fn back_to_composing(&mut self) {
        self.mode = Mode::Composing;
        self.candidates.clear();
        self.highlighted = 0;
        self.refresh_preedit();
    }

    fn clear_all(&mut self) {
        self.raw.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.mode = Mode::Composing;
        self.slots.clear(); // drop any abandoned in-flight AI requests
        self.refresh_preedit();
    }

    /// Commit the current composition exactly as displayed (raw romaji in AI
    /// mode, kana offline) — no AI conversion. Used by Enter, and by Space when
    /// AI is unavailable.
    fn commit_composition(&mut self) -> u32 {
        // If an auto-convert preview was showing, signal CANDIDATES so the
        // frontend tears the suggestion list down along with the commit.
        let had_preview = !self.candidates.is_empty();
        self.commit = self.preedit.clone();
        self.clear_all();
        let mut f = flags::CONSUMED | flags::PREEDIT | flags::COMMIT;
        if had_preview {
            f |= flags::CANDIDATES;
        }
        f
    }

    fn commit_candidate(&mut self, index: usize) -> u32 {
        if let Some(text) = self.candidates.get(index).cloned() {
            // Learn this choice for the current reading so it ranks first later.
            if let Ok(mut learning) = self.learning.lock() {
                learning.record(&self.raw, &text);
            }
            self.commit = text;
            self.clear_all();
            flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES | flags::COMMIT
        } else {
            0
        }
    }

    // --- key handling ----------------------------------------------------

    pub fn process_key(&mut self, key: Key) -> u32 {
        self.commit.clear();
        match self.mode {
            Mode::Candidates => self.process_key_candidates(key),
            Mode::Composing => self.process_key_composing(key),
        }
    }

    fn process_key_composing(&mut self, key: Key) -> u32 {
        if key.sym == keysym::SPACE {
            if self.raw.is_empty() {
                return 0;
            }
            if !self.candidates.is_empty() {
                // An auto-convert preview is showing: Space *engages* it (switches
                // to candidate selection) rather than re-running the conversion.
                // The frontend reaches here because begin_ai_convert returns None
                // while a preview is present, so it falls through to process_key.
                self.mode = Mode::Candidates;
                self.highlighted = 0;
                self.refresh_preedit();
                return flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES;
            }
            // No preview to engage (AI unavailable): commit as displayed. The
            // frontend prefers AI by calling begin_ai_convert on Space first; this
            // is only reached when there is no converter.
            return self.commit_composition();
        }
        if let Some(c) = key.printable_char() {
            // Typing dismisses any stale auto-convert preview and resumes editing.
            let had_preview = !self.candidates.is_empty();
            self.raw.push(c);
            if had_preview {
                self.candidates.clear();
                self.highlighted = 0;
            }
            self.refresh_preedit();
            return flags::CONSUMED | flags::PREEDIT | preview_flag(had_preview);
        }
        match key.sym {
            keysym::BACKSPACE => {
                let had_preview = !self.candidates.is_empty();
                if self.raw.pop().is_some() {
                    if had_preview {
                        self.candidates.clear();
                        self.highlighted = 0;
                    }
                    self.refresh_preedit();
                    flags::CONSUMED | flags::PREEDIT | preview_flag(had_preview)
                } else {
                    0
                }
            }
            keysym::RETURN => {
                if self.raw.is_empty() {
                    0
                } else {
                    // Commits the preedit, which in Composing is the raw romaji —
                    // never the AI preview. This is the "Enter = as-typed" half of
                    // the "Space converts, Enter commits what you see" model.
                    self.commit_composition()
                }
            }
            keysym::ESCAPE => {
                if !self.candidates.is_empty() {
                    // First Esc dismisses the auto-convert preview, keeping the
                    // romaji so the user can keep editing.
                    self.candidates.clear();
                    self.highlighted = 0;
                    self.refresh_preedit();
                    flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES
                } else if !self.raw.is_empty() {
                    self.raw.clear();
                    self.refresh_preedit();
                    flags::CONSUMED | flags::PREEDIT
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn process_key_candidates(&mut self, key: Key) -> u32 {
        let len = self.candidates.len();
        if len == 0 {
            self.back_to_composing();
            return flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES;
        }
        // Number keys 1..=9 select directly.
        if let Some(c) = key.printable_char() {
            if let Some(d) = c.to_digit(10) {
                if d >= 1 && (d as usize) <= len {
                    return self.commit_candidate(d as usize - 1);
                }
            }
            // Any other character commits the highlighted candidate and starts a
            // fresh composition with that character.
            let flags_commit = self.commit_candidate(self.highlighted);
            self.raw.push(c);
            self.refresh_preedit();
            return flags_commit | flags::PREEDIT;
        }
        match key.sym {
            keysym::SPACE | keysym::DOWN => {
                self.highlighted = (self.highlighted + 1) % len;
                self.refresh_preedit();
                flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES
            }
            keysym::UP => {
                self.highlighted = (self.highlighted + len - 1) % len;
                self.refresh_preedit();
                flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES
            }
            keysym::RETURN => self.commit_candidate(self.highlighted),
            keysym::ESCAPE | keysym::BACKSPACE => {
                // Cancel conversion, keep the romaji for further editing.
                self.back_to_composing();
                flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES
            }
            _ => 0,
        }
    }

    /// Commit the candidate at `index` (used by the C ABI / IPC select path).
    pub fn select_candidate(&mut self, index: usize) -> u32 {
        self.commit.clear();
        self.commit_candidate(index)
    }

    pub fn reset(&mut self) -> u32 {
        self.commit.clear();
        self.clear_all();
        flags::PREEDIT | flags::CANDIDATES
    }

    /// Append the raw romaji (as-typed lowercase, then uppercased) to the AI
    /// candidate list as last-resort "exactly what I typed" choices. Skips empties
    /// and duplicates.
    fn with_romaji_fallbacks(mut candidates: Vec<String>, raw: &str) -> Vec<String> {
        for extra in [raw.to_string(), raw.to_uppercase()] {
            if !extra.is_empty() && !candidates.contains(&extra) {
                candidates.push(extra);
            }
        }
        candidates
    }

    // --- cloud-AI conversion (async) ------------------------------------

    /// Start an asynchronous cloud-AI conversion of the current romaji, with the
    /// surrounding document text as context. Returns a request id to poll, or
    /// `None` if AI is unavailable, nothing is being composed, or candidates are
    /// already showing.
    pub fn begin_ai_convert(
        &mut self,
        explicit: bool,
        context_before: String,
        context_after: String,
    ) -> Option<u64> {
        let converter = self.converter.clone()?;
        // Nothing to convert, already selecting candidates, or a preview is
        // already showing (Space should *engage* that preview via process_key,
        // not kick off another conversion). Returning None makes the frontend
        // fall through to process_key for the engage/cycle path.
        if self.mode != Mode::Composing || self.raw.is_empty() || !self.candidates.is_empty() {
            return None;
        }
        self.convert_explicit = explicit;
        let req = ConvertRequest {
            romaji: self.raw.clone(),
            kana: romaji::flush(&self.raw),
            context_before,
            context_after,
        };
        let id = self.next_req;
        self.next_req += 1;
        let slot = Arc::new(Mutex::new(Slot::default()));
        self.slots.insert(id, slot.clone());

        std::thread::spawn(move || {
            // Stream partial candidates into the slot as the model generates them.
            let result = {
                let mut sink = |cands: Vec<String>| {
                    if let Ok(mut s) = slot.lock() {
                        s.candidates = cands;
                    }
                };
                converter.convert_streaming(&req, &mut sink)
            };
            if let Ok(mut s) = slot.lock() {
                match result {
                    Ok(()) => s.done = true,
                    Err(e) => {
                        s.error = Some(e.to_string());
                        s.done = true;
                    }
                }
            }
        });
        Some(id)
    }

    /// Poll a conversion started by [`Session::begin_ai_convert`]. Returns
    /// [`AiPoll::Streaming`] once partial candidates are available (and more may
    /// arrive), [`AiPoll::Ready`] when finished, or [`AiPoll::Error`]. On the
    /// first non-empty result an *explicit* conversion enters candidate mode; an
    /// auto-convert shows a preview (stays composing — see `convert_explicit`).
    pub fn poll_ai_result(&mut self, req_id: u64) -> AiPoll {
        let slot = match self.slots.get(&req_id) {
            Some(s) => s.clone(),
            None => return AiPoll::Error("unknown request".to_owned()),
        };
        let (cands, done, error) = {
            let guard = slot.lock().unwrap();
            (guard.candidates.clone(), guard.done, guard.error.clone())
        };

        if let Some(e) = error {
            self.slots.remove(&req_id);
            self.last_error = e.clone();
            return AiPoll::Error(e);
        }
        if cands.is_empty() {
            if done {
                self.slots.remove(&req_id);
                return AiPoll::Error("no candidates".to_owned());
            }
            return AiPoll::Pending;
        }

        // Candidates available (partial or final): show them now, with the raw
        // romaji (as-typed and uppercased) appended at the bottom as a guaranteed
        // "exactly what I typed" choice.
        self.candidates = Self::with_romaji_fallbacks(cands, &self.raw);
        // Promote previously-chosen candidates for this reading to the top.
        if let Ok(learning) = self.learning.lock() {
            learning.reorder(&self.raw, &mut self.candidates);
        }
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
        // Explicit (Space) conversions engage candidate selection immediately, so
        // Enter commits the chosen candidate. Auto-convert (typing pause) shows a
        // non-committal PREVIEW: stay in Composing so the preedit remains the raw
        // romaji and Enter commits as-typed; the user presses Space to engage.
        if self.convert_explicit {
            self.mode = Mode::Candidates;
        } else {
            self.highlighted = 0; // preview points at the top suggestion
        }
        self.refresh_preedit();
        if done {
            self.slots.remove(&req_id);
            AiPoll::Ready
        } else {
            AiPoll::Streaming
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiError;
    use crate::key::{keysym, Key};

    fn fresh_learning() -> Arc<Mutex<Learning>> {
        Arc::new(Mutex::new(Learning::load(None))) // in-memory, no persistence
    }

    fn no_ai() -> Session {
        Session::new(None, fresh_learning())
    }

    fn type_str(s: &mut Session, text: &str) {
        for c in text.chars() {
            s.process_key(Key::new(c as u32, 0));
        }
    }

    // ---- composing-mode behaviour (unchanged from M1) -------------------

    #[test]
    fn romaji_becomes_kana() {
        let mut s = no_ai();
        type_str(&mut s, "konnichiha");
        assert_eq!(s.preedit(), "こんにちは");
    }

    #[test]
    fn enter_commits_kana() {
        let mut s = no_ai();
        type_str(&mut s, "hon");
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "ほん");
    }

    #[test]
    fn space_commits_kana_when_no_ai() {
        let mut s = no_ai();
        type_str(&mut s, "ka");
        let f = s.process_key(Key::new(keysym::SPACE, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "か");
    }

    #[test]
    fn begin_ai_returns_none_without_converter() {
        let mut s = no_ai();
        type_str(&mut s, "ka");
        assert_eq!(s.begin_ai_convert(true, String::new(), String::new()), None);
    }

    // ---- cloud-AI conversion with a mock converter ----------------------

    struct MockConverter {
        candidates: Vec<String>,
    }
    impl Converter for MockConverter {
        fn convert(&self, _req: &ConvertRequest) -> Result<Vec<String>, AiError> {
            Ok(self.candidates.clone())
        }
    }

    struct FailingConverter;
    impl Converter for FailingConverter {
        fn convert(&self, _req: &ConvertRequest) -> Result<Vec<String>, AiError> {
            Err(AiError::Network("boom".into()))
        }
    }

    fn with_mock(candidates: &[&str]) -> Session {
        Session::new(
            Some(Arc::new(MockConverter {
                candidates: candidates.iter().map(|s| s.to_string()).collect(),
            })),
            fresh_learning(),
        )
    }

    /// Poll until the background conversion resolves (bounded).
    fn poll_until_done(s: &mut Session, id: u64) -> AiPoll {
        for _ in 0..400 {
            match s.poll_ai_result(id) {
                AiPoll::Pending | AiPoll::Streaming => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                done => return done,
            }
        }
        panic!("conversion did not resolve");
    }

    #[test]
    fn ai_convert_populates_candidates_and_enters_candidate_mode() {
        let mut s = with_mock(&["日本語", "にほんご", "二本後"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, "私は".into(), "が好き".into())
            .unwrap();
        assert_eq!(poll_until_done(&mut s, id), AiPoll::Ready);
        // AI candidates, then the raw romaji (as-typed + uppercased) at the bottom.
        assert_eq!(
            s.candidates(),
            &["日本語", "にほんご", "二本後", "nihongo", "NIHONGO"]
        );
        assert_eq!(s.preedit(), "日本語"); // highlighted candidate shown inline
    }

    #[test]
    fn learning_promotes_previously_selected() {
        let mut s = with_mock(&["日本語", "にほんご", "二本後"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        assert_eq!(s.candidates()[0], "日本語"); // default order first time

        s.process_key(Key::new(keysym::SPACE, 0)); // highlight 1 -> にほんご
        let f = s.process_key(Key::new(keysym::RETURN, 0)); // commit にほんご (learned)
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "にほんご");

        // Same reading again -> the learned candidate is now first.
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        assert_eq!(s.candidates()[0], "にほんご");
    }

    #[test]
    fn romaji_fallbacks_dedup_when_ai_returns_same() {
        let mut s = with_mock(&["nihongo"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        // lowercase already present -> skipped; only the uppercase is appended.
        assert_eq!(s.candidates(), &["nihongo", "NIHONGO"]);
    }

    #[test]
    fn space_cycles_candidates_enter_commits() {
        let mut s = with_mock(&["日本語", "にほんご"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);

        s.process_key(Key::new(keysym::SPACE, 0)); // -> highlight 1
        assert_eq!(s.preedit(), "にほんご");
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "にほんご");
        assert!(s.is_empty());
    }

    #[test]
    fn number_key_selects_candidate() {
        let mut s = with_mock(&["一", "二", "三"]);
        type_str(&mut s, "ichi");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        let f = s.process_key(Key::new('2' as u32, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "二");
    }

    #[test]
    fn escape_cancels_back_to_romaji() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        s.process_key(Key::new(keysym::ESCAPE, 0));
        // Back to composing: the romaji is preserved (AI mode shows raw romaji).
        assert_eq!(s.preedit(), "nihongo");
        // And a second conversion can be started.
        assert!(s
            .begin_ai_convert(true, String::new(), String::new())
            .is_some());
    }

    #[test]
    fn begin_ai_returns_none_in_candidate_mode() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        // Already showing candidates -> no new conversion (frontend will cycle).
        assert_eq!(s.begin_ai_convert(true, String::new(), String::new()), None);
    }

    #[test]
    fn ai_failure_is_reported() {
        let mut s = Session::new(Some(Arc::new(FailingConverter)), fresh_learning());
        type_str(&mut s, "ka");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        assert!(matches!(poll_until_done(&mut s, id), AiPoll::Error(_)));
        // Still composing (AI mode shows raw romaji); frontend can retry/fallback.
        assert_eq!(s.preedit(), "ka");
    }

    #[test]
    fn typing_in_candidate_mode_commits_then_continues() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(true, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        // Typing 'k' commits 日本語 and starts a new composition.
        let f = s.process_key(Key::new('k' as u32, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "日本語");
        s.process_key(Key::new('a' as u32, 0));
        assert_eq!(s.preedit(), "ka"); // AI mode shows raw romaji
    }

    #[test]
    fn enter_commits_raw_romaji_in_ai_mode_no_conversion() {
        // In AI mode, Enter commits exactly what's shown (raw romaji), without
        // invoking the AI. (Space / auto-convert are the AI path.)
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "github");
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "github");
        assert!(s.is_empty());
    }

    // ---- auto-convert PREVIEW (explicit = false) ------------------------
    // "Space converts, Enter commits what you see": an auto-convert (typing
    // pause) is a non-committal preview — Enter still commits the raw romaji
    // until the user presses Space to engage.

    #[test]
    fn auto_convert_preview_keeps_raw_and_enter_commits_raw() {
        let mut s = with_mock(&["日本語", "にほんご"]);
        type_str(&mut s, "nihongo");
        // explicit = false: the typing-pause path.
        let id = s
            .begin_ai_convert(false, String::new(), String::new())
            .unwrap();
        assert_eq!(poll_until_done(&mut s, id), AiPoll::Ready);
        // Candidates are previewed below…
        assert_eq!(
            s.candidates(),
            &["日本語", "にほんご", "nihongo", "NIHONGO"]
        );
        // …but the preedit is still the raw romaji (NOT the top candidate).
        assert_eq!(s.preedit(), "nihongo");
        // Enter commits exactly that — no AI.
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "nihongo");
        assert!(s.is_empty());
    }

    #[test]
    fn space_engages_preview_then_enter_commits_candidate() {
        let mut s = with_mock(&["日本語", "にほんご"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(false, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        assert_eq!(s.preedit(), "nihongo"); // preview: raw still shown

        // Space engages the preview → top candidate becomes the selection.
        let f = s.process_key(Key::new(keysym::SPACE, 0));
        assert!(f & flags::CONSUMED != 0);
        assert_eq!(s.preedit(), "日本語");
        // A further Space now cycles (we're in candidate selection).
        s.process_key(Key::new(keysym::SPACE, 0));
        assert_eq!(s.preedit(), "にほんご");
        // Enter commits the selected candidate.
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "にほんご");
    }

    #[test]
    fn typing_dismisses_preview_and_resumes_composing() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(false, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        assert!(!s.candidates().is_empty());
        // Typing another letter drops the preview and keeps composing raw romaji.
        let f = s.process_key(Key::new('u' as u32, 0));
        assert!(f & flags::CANDIDATES != 0); // signals the window to hide
        assert!(s.candidates().is_empty());
        assert_eq!(s.preedit(), "nihongou");
    }

    #[test]
    fn begin_returns_none_while_preview_showing() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s
            .begin_ai_convert(false, String::new(), String::new())
            .unwrap();
        poll_until_done(&mut s, id);
        // A preview is up: begin must return None so the frontend falls through
        // to process_key (Space → engage), instead of re-running the conversion.
        assert_eq!(s.begin_ai_convert(true, String::new(), String::new()), None);
    }

    #[test]
    fn ai_mode_shows_raw_romaji_offline_shows_kana() {
        // With a converter, composing shows the raw romaji (English-friendly).
        let mut ai = with_mock(&["x"]);
        type_str(&mut ai, "github");
        assert_eq!(ai.preedit(), "github");
        // Without a converter, composing shows kana.
        let mut off = no_ai();
        type_str(&mut off, "nihongo");
        assert_eq!(off.preedit(), "にほんご");
    }
}
