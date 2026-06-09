//! Platform-neutral key representation.
//!
//! Each frontend (TSF on Windows, IMK on macOS) translates its native key event
//! into a [`Key`] *before* calling the engine, so the engine never sees an
//! OS-specific virtual-key code. Keysym values follow the X11 / IBus convention
//! (also used by librime): printable ASCII maps to its Unicode code point, and
//! special keys live in the `0xFF00` range.

/// Named keysyms for non-printable keys (X11 convention).
pub mod keysym {
    pub const BACKSPACE: u32 = 0xFF08;
    pub const TAB: u32 = 0xFF09;
    pub const RETURN: u32 = 0xFF0D;
    pub const ESCAPE: u32 = 0xFF1B;
    pub const SPACE: u32 = 0x0020;
    pub const DELETE: u32 = 0xFFFF;
    pub const LEFT: u32 = 0xFF51;
    pub const UP: u32 = 0xFF52;
    pub const RIGHT: u32 = 0xFF53;
    pub const DOWN: u32 = 0xFF54;
}

/// Modifier bitmask (X11-style bits).
pub mod modifiers {
    pub const SHIFT: u32 = 1 << 0;
    pub const CONTROL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
}

/// Result bitflags returned by [`crate::Session::process_key`].
///
/// These exact values are part of the C ABI / IPC contract (`RimeResultFlags`),
/// so they must not be renumbered without bumping the ABI version.
pub mod flags {
    /// The IME consumed the key (the host app should not also handle it).
    pub const CONSUMED: u32 = 1 << 0;
    /// The preedit (composition) string changed.
    pub const PREEDIT: u32 = 1 << 1;
    /// The candidate list changed.
    pub const CANDIDATES: u32 = 1 << 2;
    /// There is committed text to fetch and insert into the document.
    pub const COMMIT: u32 = 1 << 3;
}

/// A single key event, neutral across platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    /// Keysym: a Unicode code point for printable keys, or a `keysym::*` value.
    pub sym: u32,
    /// Modifier bitmask (see [`modifiers`]).
    pub mods: u32,
}

impl Key {
    pub fn new(sym: u32, mods: u32) -> Self {
        Self { sym, mods }
    }

    /// The printable character this key produces, if any.
    ///
    /// Returns `None` for control keys, for Space (which the IME treats as a
    /// command, not romaji input — see [`keysym::SPACE`]), and when Control/Alt
    /// is held (those are shortcuts, not text input).
    pub fn printable_char(&self) -> Option<char> {
        if self.mods & (modifiers::CONTROL | modifiers::ALT) != 0 {
            return None;
        }
        if (0x21..=0x7E).contains(&self.sym) {
            char::from_u32(self.sym)
        } else {
            None
        }
    }
}
