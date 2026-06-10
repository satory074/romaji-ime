//! C ABI over [`ime_engine`]. This is the **macOS contract**: the IMK `.app`
//! links this as a static library and calls these functions in-process.
//!
//! Conventions (librime `rime_api.h` style):
//!   - `RimeEngine` / `RimeSession` are opaque, heap-allocated handles.
//!   - All strings are UTF-8, NUL-terminated. Returned `const char*` are owned by
//!     the session and remain valid until the next mutating call on that session;
//!     the caller copies out immediately.
//!   - The cloud-AI conversion is asynchronous and callback-free: `begin` returns
//!     a request id and the frontend polls. (Stubbed until M2.)
//!
//! Regenerate the header after changing signatures:
//!   `cargo run -p xtask -- gen-header`

use ime_engine::{AiPoll, Engine, Key, Session};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;

/// Bump when the ABI changes in a backward-incompatible way.
const ABI_VERSION: u32 = 1;

// Result-flag bits returned by `rime_process_key` / `rime_select_candidate`,
// exported into the C header so the Swift (macOS) and C++ (Windows) frontends
// share these values. Kept in sync with `ime_engine::flags` by `flag_values`.
/// The IME consumed the key; the host app must not also handle it.
pub const RIME_CONSUMED: u32 = 1;
/// The preedit (composition) string changed.
pub const RIME_PREEDIT: u32 = 2;
/// The candidate list changed.
pub const RIME_CANDIDATES: u32 = 4;
/// There is committed text to fetch and insert.
pub const RIME_COMMIT: u32 = 8;

/// Opaque process-global engine handle.
pub struct RimeEngine {
    inner: Engine,
}

/// Opaque per-input-context session handle.
///
/// The C-string caches keep returned `const char*` pointers valid until the next
/// mutating call (`process_key` / `select_candidate` / `reset`).
pub struct RimeSession {
    inner: Session,
    preedit_c: CString,
    commit_c: CString,
    candidates_c: Vec<CString>,
    error_c: CString,
}

impl RimeSession {
    /// Rebuild the C-string caches from the engine session's current state.
    fn refresh(&mut self) {
        self.preedit_c = to_cstring(self.inner.preedit());
        self.commit_c = to_cstring(self.inner.commit_text());
        self.candidates_c = self
            .inner
            .candidates()
            .iter()
            .map(|s| to_cstring(s))
            .collect();
    }
}

/// UTF-8 -> CString, stripping interior NULs (which can't occur in our text but
/// must be handled to avoid a panic at the FFI boundary).
fn to_cstring(s: &str) -> CString {
    match CString::new(s) {
        Ok(c) => c,
        Err(_) => CString::new(s.replace('\0', "")).unwrap_or_default(),
    }
}

/// SAFETY: `p` is null or a valid NUL-terminated C string.
unsafe fn cstr_to_pathbuf(p: *const c_char) -> Option<PathBuf> {
    let s = cstr_to_string(p);
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

/// SAFETY: `p` is null or a valid NUL-terminated C string. NULL -> "".
unsafe fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create an engine. `config_dir`/`user_data_dir` may be NULL. The returned
/// pointer must be freed with [`rime_engine_free`].
#[no_mangle]
pub extern "C" fn rime_engine_new(
    config_dir: *const c_char,
    user_data_dir: *const c_char,
) -> *mut RimeEngine {
    let config = unsafe { cstr_to_pathbuf(config_dir) };
    let user = unsafe { cstr_to_pathbuf(user_data_dir) };
    Box::into_raw(Box::new(RimeEngine {
        // Attach the cloud-AI converter from config.json / env if configured.
        inner: Engine::new(config, user).with_ai_from_config(),
    }))
}

/// Free an engine created by [`rime_engine_new`]. NULL is ignored.
#[no_mangle]
pub extern "C" fn rime_engine_free(engine: *mut RimeEngine) {
    if !engine.is_null() {
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// Whether a cloud-AI converter is configured (config.json / env loaded). Useful
/// for diagnostics and for deciding whether to offer AI conversion.
#[no_mangle]
pub extern "C" fn rime_engine_has_ai(engine: *const RimeEngine) -> bool {
    unsafe { engine.as_ref() }
        .map(|e| e.inner.has_ai())
        .unwrap_or(false)
}

/// Whether the frontend should auto-convert after a typing pause (config).
#[no_mangle]
pub extern "C" fn rime_auto_convert_enabled(engine: *const RimeEngine) -> bool {
    unsafe { engine.as_ref() }
        .map(|e| e.inner.auto_convert_enabled())
        .unwrap_or(true)
}

/// Idle delay in milliseconds before auto-converting (config).
#[no_mangle]
pub extern "C" fn rime_auto_convert_delay_ms(engine: *const RimeEngine) -> u32 {
    unsafe { engine.as_ref() }
        .map(|e| e.inner.auto_convert_delay_ms())
        .unwrap_or(500)
}

/// Start a new input session. Returns NULL if `engine` is NULL. Free with
/// [`rime_session_free`]. The engine must outlive all its sessions.
#[no_mangle]
pub extern "C" fn rime_session_new(engine: *mut RimeEngine) -> *mut RimeSession {
    let engine = match unsafe { engine.as_ref() } {
        Some(e) => e,
        None => return std::ptr::null_mut(),
    };
    let mut session = Box::new(RimeSession {
        inner: engine.inner.new_session(),
        preedit_c: CString::default(),
        commit_c: CString::default(),
        candidates_c: Vec::new(),
        error_c: CString::default(),
    });
    session.refresh();
    Box::into_raw(session)
}

/// Free a session created by [`rime_session_new`]. NULL is ignored.
#[no_mangle]
pub extern "C" fn rime_session_free(session: *mut RimeSession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// Clear all composition state for the session.
#[no_mangle]
pub extern "C" fn rime_session_reset(session: *mut RimeSession) {
    if let Some(s) = unsafe { session.as_mut() } {
        s.inner.reset();
        s.refresh();
    }
}

// ---------------------------------------------------------------------------
// Key processing & read-back
// ---------------------------------------------------------------------------

/// Feed one platform-neutral key event. Returns a `RimeResultFlags` bitmask
/// (see the engine's `flags` module): CONSUMED=1, PREEDIT=2, CANDIDATES=4,
/// COMMIT=8. Returns 0 if `session` is NULL.
#[no_mangle]
pub extern "C" fn rime_process_key(session: *mut RimeSession, keysym: u32, mods: u32) -> u32 {
    let s = match unsafe { session.as_mut() } {
        Some(s) => s,
        None => return 0,
    };
    let flags = s.inner.process_key(Key::new(keysym, mods));
    s.refresh();
    flags
}

/// The current preedit (composition) string. Valid until the next mutating call.
#[no_mangle]
pub extern "C" fn rime_get_preedit(session: *const RimeSession) -> *const c_char {
    match unsafe { session.as_ref() } {
        Some(s) => s.preedit_c.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Number of conversion candidates.
#[no_mangle]
pub extern "C" fn rime_get_candidate_count(session: *const RimeSession) -> usize {
    unsafe { session.as_ref() }
        .map(|s| s.candidates_c.len())
        .unwrap_or(0)
}

/// The candidate string at `index`, or NULL if out of range. Valid until the
/// next mutating call.
#[no_mangle]
pub extern "C" fn rime_get_candidate_text(
    session: *const RimeSession,
    index: usize,
) -> *const c_char {
    match unsafe { session.as_ref() } {
        Some(s) => s
            .candidates_c
            .get(index)
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null()),
        None => std::ptr::null(),
    }
}

/// Index of the highlighted candidate.
#[no_mangle]
pub extern "C" fn rime_get_highlighted_index(session: *const RimeSession) -> usize {
    unsafe { session.as_ref() }
        .map(|s| s.inner.highlighted())
        .unwrap_or(0)
}

/// Commit the candidate at `index`. Returns a result-flags bitmask.
#[no_mangle]
pub extern "C" fn rime_select_candidate(session: *mut RimeSession, index: usize) -> u32 {
    match unsafe { session.as_mut() } {
        Some(s) => {
            let flags = s.inner.select_candidate(index);
            s.refresh();
            flags
        }
        None => 0,
    }
}

/// Text to commit after a key event set the COMMIT flag. Valid until the next
/// mutating call; empty string if nothing to commit.
#[no_mangle]
pub extern "C" fn rime_get_commit_text(session: *const RimeSession) -> *const c_char {
    match unsafe { session.as_ref() } {
        Some(s) => s.commit_c.as_ptr(),
        None => std::ptr::null(),
    }
}

// ---------------------------------------------------------------------------
// Cloud-AI conversion (asynchronous, pull model). Stubbed until M2.
// ---------------------------------------------------------------------------

/// Begin an asynchronous cloud-AI conversion of the current input, passing the
/// surrounding document text as context. Returns a request id to poll, or 0 if
/// AI conversion is unavailable (no converter configured, nothing composing, or
/// candidates already showing). The conversion runs on an internal background
/// thread; call [`rime_poll_ai_result`] on the SAME thread as other session
/// calls until it resolves.
#[no_mangle]
pub extern "C" fn rime_begin_ai_convert(
    session: *mut RimeSession,
    context_before: *const c_char,
    context_after: *const c_char,
) -> u64 {
    let s = match unsafe { session.as_mut() } {
        Some(s) => s,
        None => return 0,
    };
    let before = unsafe { cstr_to_string(context_before) };
    let after = unsafe { cstr_to_string(context_after) };
    s.inner.begin_ai_convert(before, after).unwrap_or(0)
}

/// Poll a conversion started by [`rime_begin_ai_convert`].
/// Returns: 0 = pending, 1 = ready/final (candidates/preedit updated), 2 =
/// streaming (partial candidates updated, keep polling), -1 = error/unavailable
/// (the frontend should fall back to the local converter).
#[no_mangle]
pub extern "C" fn rime_poll_ai_result(session: *mut RimeSession, req_id: u64) -> i32 {
    let s = match unsafe { session.as_mut() } {
        Some(s) => s,
        None => return -1,
    };
    match s.inner.poll_ai_result(req_id) {
        AiPoll::Pending => 0,
        AiPoll::Ready => {
            s.refresh();
            1
        }
        AiPoll::Streaming => {
            s.refresh();
            2 // partial candidates available; keep polling for more
        }
        AiPoll::Error(_) => {
            s.error_c = to_cstring(s.inner.last_error());
            -1
        }
    }
}

/// The most recent AI conversion error message (empty if none). Valid until the
/// next poll. For diagnostics / user-facing error display.
#[no_mangle]
pub extern "C" fn rime_get_last_error(session: *const RimeSession) -> *const c_char {
    match unsafe { session.as_ref() } {
        Some(s) => s.error_c.as_ptr(),
        None => std::ptr::null(),
    }
}

/// The C ABI version. Frontends should check this matches what they were built
/// against.
#[no_mangle]
pub extern "C" fn rime_abi_version() -> u32 {
    ABI_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    /// Exercise the full handle lifecycle through the C ABI, mirroring what the
    /// macOS frontend does: create engine -> session -> type "ka" -> Enter ->
    /// read the converted kana commit text.
    #[test]
    fn ffi_romaji_roundtrip() {
        let engine = rime_engine_new(std::ptr::null(), std::ptr::null());
        assert!(!engine.is_null());
        let session = rime_session_new(engine);
        assert!(!session.is_null());

        rime_process_key(session, 'k' as u32, 0);
        rime_process_key(session, 'a' as u32, 0);
        let preedit = unsafe { CStr::from_ptr(rime_get_preedit(session)) };
        assert_eq!(preedit.to_str().unwrap(), "か");

        let flags = rime_process_key(session, ime_engine::keysym::RETURN, 0);
        assert!(flags & ime_engine::flags::COMMIT != 0);
        let commit = unsafe { CStr::from_ptr(rime_get_commit_text(session)) };
        assert_eq!(commit.to_str().unwrap(), "か");

        rime_session_free(session);
        rime_engine_free(engine);
    }

    #[test]
    fn flag_values() {
        // The C-ABI flag constants must mirror the engine's flags exactly.
        assert_eq!(RIME_CONSUMED, ime_engine::flags::CONSUMED);
        assert_eq!(RIME_PREEDIT, ime_engine::flags::PREEDIT);
        assert_eq!(RIME_CANDIDATES, ime_engine::flags::CANDIDATES);
        assert_eq!(RIME_COMMIT, ime_engine::flags::COMMIT);
    }

    #[test]
    fn null_handles_are_safe() {
        assert_eq!(rime_process_key(std::ptr::null_mut(), 0x61, 0), 0);
        assert!(rime_get_preedit(std::ptr::null()).is_null());
        assert_eq!(rime_get_candidate_count(std::ptr::null()), 0);
        rime_session_free(std::ptr::null_mut());
        rime_engine_free(std::ptr::null_mut());
    }
}
