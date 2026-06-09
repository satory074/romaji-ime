//! OS-independent core engine for the cross-platform romaji IME.
//!
//! This crate is pure Rust with no FFI and no platform code, so it is fully
//! unit-testable on the host. The platform frontends drive it indirectly:
//!   - macOS links it in-process through the C ABI in `ime-ffi`.
//!   - Windows reaches it over a named pipe via `ime-server` + `ime-ipc`.
//!
//! Design: an [`Engine`] holds process-global state (config, and in later
//! milestones the dictionary and converters). Each input context gets its own
//! [`Session`], which is a small state machine over the preedit buffer and
//! candidate list. Key events arrive as platform-neutral [`Key`]s.
//!
//! Milestone status: **M0** — the session is a plain echo (typed ASCII goes
//! into the preedit, Enter commits it). M1 replaces the echo with real
//! romaji→kana conversion; later milestones add the dictionary, Viterbi, and the
//! cloud-AI converter behind the same [`Session`] surface.

pub mod key;
mod session;

pub use key::{flags, keysym, modifiers, Key};
pub use session::Session;

use std::path::PathBuf;

/// Process-global engine state.
///
/// In M0 this is essentially empty; it exists so the lifecycle and ownership
/// model (one engine, many sessions) is in place from the start. Later it owns
/// the loaded dictionary, cost model, converter configuration, and the shared
/// learning database.
#[derive(Debug, Default)]
pub struct Engine {
    #[allow(dead_code)]
    config_dir: Option<PathBuf>,
    #[allow(dead_code)]
    user_data_dir: Option<PathBuf>,
}

impl Engine {
    /// Create an engine. `config_dir` points at bundled read-only data
    /// (dictionary, romaji tables); `user_data_dir` is the per-user writable
    /// location (learning DB, settings). Both are optional in M0.
    pub fn new(config_dir: Option<PathBuf>, user_data_dir: Option<PathBuf>) -> Self {
        Engine {
            config_dir,
            user_data_dir,
        }
    }

    /// Start a new input session (one per focused text field / IMK controller).
    pub fn new_session(&self) -> Session {
        Session::new()
    }
}
