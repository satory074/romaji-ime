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
//! Milestone status: **M1** — the session converts romaji to hiragana
//! incrementally (see [`romaji`]); Enter/Space commit the kana. Later milestones
//! add the dictionary, Viterbi, and the cloud-AI converter behind the same
//! [`Session`] surface.

pub mod ai;
pub mod key;
pub mod romaji;
mod session;

pub use ai::{AiPoll, ConvertRequest, Converter};
pub use key::{flags, keysym, modifiers, Key};
pub use session::Session;

use std::path::PathBuf;
use std::sync::Arc;

/// Process-global engine state.
///
/// In M0 this is essentially empty; it exists so the lifecycle and ownership
/// model (one engine, many sessions) is in place from the start. Later it owns
/// the loaded dictionary, cost model, converter configuration, and the shared
/// learning database.
#[derive(Clone)]
pub struct Engine {
    #[allow(dead_code)]
    config_dir: Option<PathBuf>,
    user_data_dir: Option<PathBuf>,
    /// Cloud-AI converter, if configured. Shared (Arc) so every session can hand
    /// it to a background thread.
    converter: Option<Arc<dyn Converter>>,
    /// Auto-convert behaviour (read from config; defaults on/500ms).
    ac_enabled: bool,
    ac_delay_ms: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            config_dir: None,
            user_data_dir: None,
            converter: None,
            ac_enabled: true,
            ac_delay_ms: 500,
        }
    }
}

impl Engine {
    /// Create an engine. `config_dir` points at bundled read-only data
    /// (dictionary, romaji tables); `user_data_dir` is the per-user writable
    /// location (settings `config.json`, and later the learning DB).
    ///
    /// This is pure (no I/O). Frontends attach a cloud-AI converter explicitly,
    /// typically via [`Engine::with_ai_from_config`].
    pub fn new(config_dir: Option<PathBuf>, user_data_dir: Option<PathBuf>) -> Self {
        Engine {
            config_dir,
            user_data_dir,
            ..Engine::default()
        }
    }

    /// Attach a specific converter (used by tests / embedders).
    pub fn with_converter(mut self, converter: Arc<dyn Converter>) -> Self {
        self.converter = Some(converter);
        self
    }

    /// Load the cloud-AI converter and behaviour settings from
    /// `{user_data_dir}/config.json` (falling back to `ROMAJI_IME_*` env).
    pub fn with_ai_from_config(mut self) -> Self {
        if self.converter.is_none() {
            self.converter = ai::converter_from_config(self.user_data_dir.as_deref());
        }
        let settings = ai::settings_from_config(self.user_data_dir.as_deref());
        self.ac_enabled = settings.auto_convert;
        self.ac_delay_ms = settings.auto_convert_delay_ms as u32;
        self
    }

    /// Whether cloud-AI conversion is available.
    pub fn has_ai(&self) -> bool {
        self.converter.is_some()
    }

    /// Whether to auto-convert after a typing pause.
    pub fn auto_convert_enabled(&self) -> bool {
        self.ac_enabled
    }

    /// Idle delay (ms) before auto-converting.
    pub fn auto_convert_delay_ms(&self) -> u32 {
        self.ac_delay_ms
    }

    /// Start a new input session (one per focused text field / IMK controller).
    pub fn new_session(&self) -> Session {
        Session::new(self.converter.clone())
    }
}
