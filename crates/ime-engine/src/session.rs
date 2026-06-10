//! Per-input-context session state machine.
//!
//! Two modes:
//!   - **Composing**: the user types romaji, shown as hiragana (see [`romaji`]).
//!   - **Candidates**: after a cloud-AI conversion, a ranked candidate list is
//!     shown; Space/arrows cycle, Enter / number keys commit, Escape cancels.
//!
//! Cloud-AI conversion is asynchronous: [`Session::begin_ai_convert`] spawns a
//! background thread for the (slow) LLM call and returns a request id;
//! [`Session::poll_ai_result`] is called repeatedly on the *same* thread as the
//! other session calls until it reports [`AiPoll::Ready`]/[`AiPoll::Error`]. The
//! only thing that touches another thread is the HTTP call itself, which writes
//! into a shared slot — the `Session` is never accessed concurrently.

use crate::ai::{AiPoll, ConvertRequest, Converter};
use crate::key::{flags, keysym, Key};
use crate::romaji;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Composing,
    Candidates,
}

/// Result of an in-flight AI request, written by the background thread.
enum Slot {
    Pending,
    Done(Vec<String>),
    Failed(String),
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
}

impl Session {
    pub(crate) fn new(converter: Option<Arc<dyn Converter>>) -> Self {
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
        self.refresh_preedit();
    }

    fn commit_kana(&mut self) -> u32 {
        self.commit = romaji::flush(&self.raw);
        self.clear_all();
        flags::CONSUMED | flags::PREEDIT | flags::COMMIT
    }

    fn commit_candidate(&mut self, index: usize) -> u32 {
        if let Some(text) = self.candidates.get(index).cloned() {
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
            // Local fallback: commit kana. (The frontend prefers AI conversion by
            // calling begin_ai_convert first; it only reaches here when AI is
            // unavailable.)
            return if self.raw.is_empty() {
                0
            } else {
                self.commit_kana()
            };
        }
        if let Some(c) = key.printable_char() {
            self.raw.push(c);
            self.refresh_preedit();
            return flags::CONSUMED | flags::PREEDIT;
        }
        match key.sym {
            keysym::BACKSPACE => {
                if self.raw.pop().is_some() {
                    self.refresh_preedit();
                    flags::CONSUMED | flags::PREEDIT
                } else {
                    0
                }
            }
            keysym::RETURN => {
                if self.raw.is_empty() {
                    0
                } else {
                    self.commit_kana()
                }
            }
            keysym::ESCAPE => {
                if self.raw.is_empty() {
                    0
                } else {
                    self.raw.clear();
                    self.refresh_preedit();
                    flags::CONSUMED | flags::PREEDIT
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

    // --- cloud-AI conversion (async) ------------------------------------

    /// Start an asynchronous cloud-AI conversion of the current romaji, with the
    /// surrounding document text as context. Returns a request id to poll, or
    /// `None` if AI is unavailable, nothing is being composed, or candidates are
    /// already showing.
    pub fn begin_ai_convert(
        &mut self,
        context_before: String,
        context_after: String,
    ) -> Option<u64> {
        let converter = self.converter.clone()?;
        if self.mode != Mode::Composing || self.raw.is_empty() {
            return None;
        }
        let req = ConvertRequest {
            romaji: self.raw.clone(),
            kana: romaji::flush(&self.raw),
            context_before,
            context_after,
        };
        let id = self.next_req;
        self.next_req += 1;
        let slot = Arc::new(Mutex::new(Slot::Pending));
        self.slots.insert(id, slot.clone());

        std::thread::spawn(move || {
            let result = converter.convert(&req);
            let mut guard = slot.lock().unwrap();
            *guard = match result {
                Ok(cands) => Slot::Done(cands),
                Err(e) => Slot::Failed(e.to_string()),
            };
        });
        Some(id)
    }

    /// Poll a conversion started by [`Session::begin_ai_convert`]. On
    /// [`AiPoll::Ready`] the candidate list is populated and the session enters
    /// candidate mode.
    pub fn poll_ai_result(&mut self, req_id: u64) -> AiPoll {
        let slot = match self.slots.get(&req_id) {
            Some(s) => s.clone(),
            None => return AiPoll::Error("unknown request".to_owned()),
        };
        let outcome = {
            let guard = slot.lock().unwrap();
            match &*guard {
                Slot::Pending => None,
                Slot::Done(c) => Some(Ok(c.clone())),
                Slot::Failed(e) => Some(Err(e.clone())),
            }
        };
        match outcome {
            None => AiPoll::Pending,
            Some(Ok(cands)) => {
                self.slots.remove(&req_id);
                if cands.is_empty() {
                    AiPoll::Error("no candidates".to_owned())
                } else {
                    self.candidates = cands;
                    self.highlighted = 0;
                    self.mode = Mode::Candidates;
                    self.refresh_preedit();
                    AiPoll::Ready
                }
            }
            Some(Err(e)) => {
                self.slots.remove(&req_id);
                self.last_error = e.clone();
                AiPoll::Error(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiError;
    use crate::key::{keysym, Key};

    fn no_ai() -> Session {
        Session::new(None)
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
        assert_eq!(s.begin_ai_convert(String::new(), String::new()), None);
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
        Session::new(Some(Arc::new(MockConverter {
            candidates: candidates.iter().map(|s| s.to_string()).collect(),
        })))
    }

    /// Poll until the background conversion resolves (bounded).
    fn poll_until_done(s: &mut Session, id: u64) -> AiPoll {
        for _ in 0..200 {
            match s.poll_ai_result(id) {
                AiPoll::Pending => std::thread::sleep(std::time::Duration::from_millis(5)),
                done => return done,
            }
        }
        panic!("conversion did not resolve");
    }

    #[test]
    fn ai_convert_populates_candidates_and_enters_candidate_mode() {
        let mut s = with_mock(&["日本語", "にほんご", "二本後"]);
        type_str(&mut s, "nihongo");
        let id = s.begin_ai_convert("私は".into(), "が好き".into()).unwrap();
        assert_eq!(poll_until_done(&mut s, id), AiPoll::Ready);
        assert_eq!(s.candidates(), &["日本語", "にほんご", "二本後"]);
        assert_eq!(s.preedit(), "日本語"); // highlighted candidate shown inline
    }

    #[test]
    fn space_cycles_candidates_enter_commits() {
        let mut s = with_mock(&["日本語", "にほんご"]);
        type_str(&mut s, "nihongo");
        let id = s.begin_ai_convert(String::new(), String::new()).unwrap();
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
        let id = s.begin_ai_convert(String::new(), String::new()).unwrap();
        poll_until_done(&mut s, id);
        let f = s.process_key(Key::new('2' as u32, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "二");
    }

    #[test]
    fn escape_cancels_back_to_romaji() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s.begin_ai_convert(String::new(), String::new()).unwrap();
        poll_until_done(&mut s, id);
        s.process_key(Key::new(keysym::ESCAPE, 0));
        // Back to composing: the romaji is preserved and shown as kana again.
        assert_eq!(s.preedit(), "にほんご");
        // And a second conversion can be started.
        assert!(s.begin_ai_convert(String::new(), String::new()).is_some());
    }

    #[test]
    fn begin_ai_returns_none_in_candidate_mode() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s.begin_ai_convert(String::new(), String::new()).unwrap();
        poll_until_done(&mut s, id);
        // Already showing candidates -> no new conversion (frontend will cycle).
        assert_eq!(s.begin_ai_convert(String::new(), String::new()), None);
    }

    #[test]
    fn ai_failure_is_reported() {
        let mut s = Session::new(Some(Arc::new(FailingConverter)));
        type_str(&mut s, "ka");
        let id = s.begin_ai_convert(String::new(), String::new()).unwrap();
        assert!(matches!(poll_until_done(&mut s, id), AiPoll::Error(_)));
        // Still composing, so the frontend can fall back to local kana.
        assert_eq!(s.preedit(), "か");
    }

    #[test]
    fn typing_in_candidate_mode_commits_then_continues() {
        let mut s = with_mock(&["日本語"]);
        type_str(&mut s, "nihongo");
        let id = s.begin_ai_convert(String::new(), String::new()).unwrap();
        poll_until_done(&mut s, id);
        // Typing 'k' commits 日本語 and starts a new composition.
        let f = s.process_key(Key::new('k' as u32, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "日本語");
        s.process_key(Key::new('a' as u32, 0));
        assert_eq!(s.preedit(), "か");
    }
}
