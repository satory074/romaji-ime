//! Per-input-context session state machine.
//!
//! **M1**: the session holds the raw romaji buffer the user has typed and shows
//! its hiragana transliteration as the preedit (see [`crate::romaji`]). Enter and
//! Space commit the kana; Backspace edits the romaji; Escape clears. There is no
//! kanji conversion or candidate list yet — that arrives with the cloud-AI
//! converter (M2) and the local dictionary converter (M3), both of which will
//! populate `candidates` without changing this public surface.

use crate::key::{flags, keysym, Key};
use crate::romaji;

/// State for a single input context.
#[derive(Debug, Default)]
pub struct Session {
    /// Raw romaji as typed. Reconverted in full on every change.
    raw: String,
    /// Cached preedit display = kana(raw) + pending tail (a lone trailing `n`
    /// is shown as ん). Recomputed by [`Session::recompute`].
    preedit: String,
    /// Conversion candidates (empty until M2/M3).
    candidates: Vec<String>,
    /// Index of the highlighted candidate.
    highlighted: usize,
    /// Text to commit, set by the key event that raised [`flags::COMMIT`];
    /// cleared on the next key.
    commit: String,
}

impl Session {
    pub fn new() -> Self {
        Session::default()
    }

    /// The current preedit (composition) string.
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// The current candidate list.
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }

    /// Index of the highlighted candidate.
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// Text committed by the most recent key event (empty if none).
    pub fn commit_text(&self) -> &str {
        &self.commit
    }

    /// True when there is no active composition.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Recompute the cached preedit from the raw buffer.
    fn recompute(&mut self) {
        let (kana, pending) = romaji::convert(&self.raw);
        // Show a lone trailing `n` as ん (tentatively); reconverting the whole
        // buffer means a following vowel still produces a na-row syllable.
        self.preedit = if pending == "n" {
            format!("{kana}ん")
        } else {
            format!("{kana}{pending}")
        };
    }

    /// Commit the current composition (flushing romaji to kana) and clear it.
    fn commit_current(&mut self) -> u32 {
        self.commit = romaji::flush(&self.raw);
        self.raw.clear();
        self.recompute();
        flags::CONSUMED | flags::PREEDIT | flags::COMMIT
    }

    /// Process one key event and return a [`flags`] bitmask.
    pub fn process_key(&mut self, key: Key) -> u32 {
        // The commit buffer only reflects the current event.
        self.commit.clear();

        // Space is a command, not romaji input.
        if key.sym == keysym::SPACE {
            // M1: commit the kana if composing; otherwise let the app insert a
            // literal space. (M2 turns this into the AI-conversion trigger.)
            return if self.raw.is_empty() {
                0
            } else {
                self.commit_current()
            };
        }

        if let Some(c) = key.printable_char() {
            self.raw.push(c);
            self.recompute();
            return flags::CONSUMED | flags::PREEDIT;
        }

        match key.sym {
            keysym::BACKSPACE => {
                if self.raw.pop().is_some() {
                    self.recompute();
                    flags::CONSUMED | flags::PREEDIT
                } else {
                    0
                }
            }
            keysym::RETURN => {
                if self.raw.is_empty() {
                    0
                } else {
                    self.commit_current()
                }
            }
            keysym::ESCAPE => {
                if self.raw.is_empty() {
                    0
                } else {
                    self.raw.clear();
                    self.recompute();
                    flags::CONSUMED | flags::PREEDIT
                }
            }
            _ => 0,
        }
    }

    /// Commit the candidate at `index` (no-op in M1 — no candidates yet).
    pub fn select_candidate(&mut self, index: usize) -> u32 {
        match self.candidates.get(index) {
            Some(text) => {
                self.commit = text.clone();
                self.raw.clear();
                self.candidates.clear();
                self.highlighted = 0;
                self.recompute();
                flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES | flags::COMMIT
            }
            None => 0,
        }
    }

    /// Clear all composition state.
    pub fn reset(&mut self) -> u32 {
        self.raw.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.commit.clear();
        self.recompute();
        flags::PREEDIT | flags::CANDIDATES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{keysym, Key};

    fn type_str(s: &mut Session, text: &str) {
        for c in text.chars() {
            s.process_key(Key::new(c as u32, 0));
        }
    }

    #[test]
    fn romaji_becomes_kana_in_preedit() {
        let mut s = Session::new();
        type_str(&mut s, "ka");
        assert_eq!(s.preedit(), "か");
        type_str(&mut s, "ki");
        assert_eq!(s.preedit(), "かき");
    }

    #[test]
    fn the_m1_demo() {
        let mut s = Session::new();
        type_str(&mut s, "konnichiha");
        assert_eq!(s.preedit(), "こんにちは");
    }

    #[test]
    fn trailing_n_shows_as_kana_in_preedit() {
        let mut s = Session::new();
        type_str(&mut s, "hon");
        assert_eq!(s.preedit(), "ほん");
        // a following vowel reinterprets it (whole-buffer reconvert).
        type_str(&mut s, "a");
        assert_eq!(s.preedit(), "ほな");
    }

    #[test]
    fn partial_romaji_shows_as_latin_tail() {
        let mut s = Session::new();
        type_str(&mut s, "ky");
        assert_eq!(s.preedit(), "ky");
        s.process_key(Key::new('a' as u32, 0));
        assert_eq!(s.preedit(), "きゃ");
    }

    #[test]
    fn backspace_edits_romaji() {
        let mut s = Session::new();
        type_str(&mut s, "kya");
        assert_eq!(s.preedit(), "きゃ");
        // Backspace drops one raw romaji char: "kya" -> "ky", which is an
        // incomplete cluster shown as a latin tail.
        s.process_key(Key::new(keysym::BACKSPACE, 0));
        assert_eq!(s.preedit(), "ky");
        s.process_key(Key::new(keysym::BACKSPACE, 0));
        assert_eq!(s.preedit(), "k");
    }

    #[test]
    fn enter_commits_flushed_kana() {
        let mut s = Session::new();
        type_str(&mut s, "hon");
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "ほん"); // trailing n flushed to ん
        assert_eq!(s.preedit(), "");
    }

    #[test]
    fn space_commits_when_composing_else_passes_through() {
        let mut s = Session::new();
        // empty: space is not consumed (app inserts a literal space)
        assert_eq!(s.process_key(Key::new(keysym::SPACE, 0)), 0);
        // composing: space commits the kana
        type_str(&mut s, "ka");
        let f = s.process_key(Key::new(keysym::SPACE, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "か");
    }

    #[test]
    fn escape_clears_composition() {
        let mut s = Session::new();
        type_str(&mut s, "konnichiha");
        let f = s.process_key(Key::new(keysym::ESCAPE, 0));
        assert_eq!(s.preedit(), "");
        assert!(f & flags::PREEDIT != 0);
    }

    #[test]
    fn control_modified_key_is_ignored() {
        let mut s = Session::new();
        let f = s.process_key(Key::new('a' as u32, crate::key::modifiers::CONTROL));
        assert_eq!(f, 0);
        assert_eq!(s.preedit(), "");
    }
}
