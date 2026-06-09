//! Per-input-context session state machine.
//!
//! M0 behaviour is a plain **echo**: printable keys append to the preedit,
//! Backspace deletes, Enter commits, Escape clears. This exists to prove the
//! full plumbing (FFI/IPC, registration, preedit/commit rendering) end-to-end on
//! both OSes before any Japanese-conversion logic is written. M1 swaps the echo
//! for romaji→kana; the public surface stays the same so the frontends don't
//! change.

use crate::key::{flags, keysym, Key};

/// State for a single input context.
#[derive(Debug, Default)]
pub struct Session {
    /// The composition string shown underlined in the host app.
    preedit: String,
    /// Conversion candidates (empty until M2).
    candidates: Vec<String>,
    /// Index of the highlighted candidate.
    highlighted: usize,
    /// Text to commit into the document. Only meaningful right after a
    /// `process_key` that set [`flags::COMMIT`]; cleared on the next key.
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

    /// Process one key event and return a [`flags`] bitmask describing what
    /// changed and whether the key was consumed.
    pub fn process_key(&mut self, key: Key) -> u32 {
        // The commit buffer only reflects the current event.
        self.commit.clear();

        if let Some(c) = key.printable_char() {
            self.preedit.push(c);
            return flags::CONSUMED | flags::PREEDIT;
        }

        match key.sym {
            keysym::BACKSPACE => {
                if self.preedit.pop().is_some() {
                    flags::CONSUMED | flags::PREEDIT
                } else {
                    0
                }
            }
            keysym::RETURN => {
                if self.preedit.is_empty() {
                    0
                } else {
                    self.commit = std::mem::take(&mut self.preedit);
                    flags::CONSUMED | flags::PREEDIT | flags::COMMIT
                }
            }
            keysym::ESCAPE => {
                if self.preedit.is_empty() {
                    0
                } else {
                    self.preedit.clear();
                    flags::CONSUMED | flags::PREEDIT
                }
            }
            _ => 0,
        }
    }

    /// Commit the candidate at `index` (no-op in M0 — no candidates yet).
    pub fn select_candidate(&mut self, index: usize) -> u32 {
        match self.candidates.get(index) {
            Some(text) => {
                self.commit = text.clone();
                self.preedit.clear();
                self.candidates.clear();
                self.highlighted = 0;
                flags::CONSUMED | flags::PREEDIT | flags::CANDIDATES | flags::COMMIT
            }
            None => 0,
        }
    }

    /// Clear all composition state.
    pub fn reset(&mut self) -> u32 {
        self.preedit.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.commit.clear();
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
    fn printable_keys_build_preedit() {
        let mut s = Session::new();
        let f = s.process_key(Key::new('k' as u32, 0));
        assert_eq!(s.preedit(), "k");
        assert!(f & flags::CONSUMED != 0);
        assert!(f & flags::PREEDIT != 0);
        type_str(&mut s, "a");
        assert_eq!(s.preedit(), "ka");
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut s = Session::new();
        type_str(&mut s, "abc");
        let f = s.process_key(Key::new(keysym::BACKSPACE, 0));
        assert_eq!(s.preedit(), "ab");
        assert!(f & flags::PREEDIT != 0);
    }

    #[test]
    fn backspace_on_empty_is_not_consumed() {
        let mut s = Session::new();
        assert_eq!(s.process_key(Key::new(keysym::BACKSPACE, 0)), 0);
    }

    #[test]
    fn enter_commits_preedit() {
        let mut s = Session::new();
        type_str(&mut s, "hi");
        let f = s.process_key(Key::new(keysym::RETURN, 0));
        assert!(f & flags::COMMIT != 0);
        assert_eq!(s.commit_text(), "hi");
        assert_eq!(s.preedit(), "");
    }

    #[test]
    fn enter_on_empty_is_passed_through() {
        let mut s = Session::new();
        assert_eq!(s.process_key(Key::new(keysym::RETURN, 0)), 0);
    }

    #[test]
    fn commit_buffer_clears_on_next_key() {
        let mut s = Session::new();
        type_str(&mut s, "hi");
        s.process_key(Key::new(keysym::RETURN, 0));
        assert_eq!(s.commit_text(), "hi");
        s.process_key(Key::new('x' as u32, 0));
        assert_eq!(s.commit_text(), "");
    }

    #[test]
    fn escape_clears_preedit() {
        let mut s = Session::new();
        type_str(&mut s, "hi");
        let f = s.process_key(Key::new(keysym::ESCAPE, 0));
        assert_eq!(s.preedit(), "");
        assert!(f & flags::PREEDIT != 0);
    }

    #[test]
    fn control_modified_key_is_not_text() {
        let mut s = Session::new();
        // Ctrl+a should not insert 'a'.
        let f = s.process_key(Key::new('a' as u32, crate::key::modifiers::CONTROL));
        assert_eq!(f, 0);
        assert_eq!(s.preedit(), "");
    }
}
