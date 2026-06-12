//! IPC contract between the Windows TSF DLL (a thin, in-every-process client) and
//! `ime-server` (one privileged process per user session that hosts the engine).
//!
//! Why a separate process on Windows: a TSF text service DLL is loaded into every
//! application, so it must stay tiny and crash-proof and must never block the
//! host's input thread. All the real work — dictionary I/O, Viterbi decoding, and
//! the slow cloud-LLM round-trip — happens in `ime-server`, reached over a named
//! pipe.
//!
//! ## Wire format
//! Each message is `[u32 length little-endian][bincode payload]`. One request maps
//! to one response, sequential per pipe. The C++ client hand-writes a matching
//! codec; the [`tests`] module pins the exact byte layout so that codec and this
//! crate can never drift silently (the plan's "byte-layout-fixed" guard).

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Reject absurd frame lengths to avoid unbounded allocation from a bad peer.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// A request from the frontend DLL to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Open a new input session; the server replies with [`Response::SessionId`].
    NewSession,
    /// Close a session and free its state.
    CloseSession { sid: u64 },
    /// Feed one platform-neutral key event.
    ProcessKey { sid: u64, keysym: u32, mods: u32 },
    /// Commit the candidate at `index`.
    SelectCandidate { sid: u64, index: u64 },
    /// Clear composition state.
    Reset { sid: u64 },
    /// Kick off an asynchronous cloud-AI conversion; reply is [`Response::AiBegun`].
    /// The current preedit plus surrounding-document context are sent to the LLM.
    /// `explicit` = true for a Space-triggered convert (engage candidate selection
    /// on completion); false for a typing-pause auto-convert (non-committal
    /// preview — preedit stays raw romaji, Enter commits as-typed until engaged).
    BeginAiConvert {
        sid: u64,
        context_before: String,
        context_after: String,
        explicit: bool,
    },
    /// Poll a previously-begun AI conversion. Reply is [`Response::State`] when
    /// ready, [`Response::Pending`] while in flight, or [`Response::Error`].
    PollAiResult { sid: u64, req_id: u64 },
}

/// A response from the server to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// A newly created session id.
    SessionId { sid: u64 },
    /// The full visible state after handling a request.
    State(State),
    /// An AI conversion request id to poll with [`Request::PollAiResult`].
    AiBegun { req_id: u64 },
    /// An AI conversion is still running.
    Pending,
    /// A generic acknowledgement (e.g. for `CloseSession`).
    Ok,
    /// Something went wrong; the frontend should fall back to local behaviour.
    Error { message: String },
}

/// The visible session state mirrored to the frontend. Field meanings match the
/// engine's session accessors; `flags` uses the same bits as `ime_engine::flags`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub flags: u32,
    pub preedit: String,
    pub commit: String,
    pub candidates: Vec<String>,
    pub highlighted: u64,
}

/// Write one length-prefixed bincode frame.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let bytes =
        bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if bytes.len() as u64 > MAX_FRAME_LEN as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME_LEN",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

/// Read one length-prefixed bincode frame.
pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME_LEN",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    bincode::deserialize(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrips() {
        let req = Request::ProcessKey {
            sid: 7,
            keysym: 0x61,
            mods: 0,
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).unwrap();
        let mut cursor = io::Cursor::new(buf);
        let got: Request = read_frame(&mut cursor).unwrap();
        assert_eq!(got, req);
    }

    /// Pins the exact wire bytes so the hand-written C++ codec can be kept
    /// byte-compatible. If this fails, the IPC protocol changed — update
    /// `docs/ipc-protocol.md` and the C++ client deliberately.
    #[test]
    fn process_key_byte_layout_is_stable() {
        // bincode (default config) encodes: enum variant tag as u32 LE, then
        // fields in declaration order with fixed-width little-endian integers.
        // Request::ProcessKey is variant index 2 (NewSession=0, CloseSession=1).
        let req = Request::ProcessKey {
            sid: 1,
            keysym: 0x61, // 'a'
            mods: 0,
        };
        let bytes = bincode::serialize(&req).unwrap();
        let expected: &[u8] = &[
            0x02, 0x00, 0x00, 0x00, // variant tag = 2
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // sid: u64 = 1
            0x61, 0x00, 0x00, 0x00, // keysym: u32 = 0x61
            0x00, 0x00, 0x00, 0x00, // mods: u32 = 0
        ];
        assert_eq!(bytes, expected);
    }

    #[test]
    fn new_session_is_variant_zero() {
        let bytes = bincode::serialize(&Request::NewSession).unwrap();
        assert_eq!(bytes, &[0x00, 0x00, 0x00, 0x00]);
    }
}
