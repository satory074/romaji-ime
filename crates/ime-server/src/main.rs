//! `ime-server`: hosts the IME engine out-of-process for the Windows frontend.
//!
//! The thin TSF DLL (loaded into every app) forwards key events here over a
//! named pipe; this process owns the engine and does all the heavy work
//! (dictionary, Viterbi, and the slow cloud-LLM round-trip) off the apps'
//! threads, which gives both crash isolation and a non-blocking input path.
//!
//! **M0**: a self-check that links the engine + IPC types and runs the echo
//! engine once, proving the crate graph builds (including cross-compiled to a
//! Windows target). The named-pipe transport lands in M1.

use ime_engine::{flags, keysym, Engine, Key};
use ime_ipc::{Request, Response, State};

/// Run the engine over a request stream. In M1 this is fed by the named-pipe
/// transport; in M0 we drive it with a fixed script to validate the wiring.
fn handle_in_memory_demo() -> State {
    let engine = Engine::new(None, None);
    let mut session = engine.new_session();

    // Type "ka", then commit with Enter (M0 engine echoes input).
    for ch in "ka".chars() {
        session.process_key(Key::new(ch as u32, 0));
    }
    let last_flags = session.process_key(Key::new(keysym::RETURN, 0));

    State {
        flags: last_flags,
        preedit: session.preedit().to_owned(),
        commit: session.commit_text().to_owned(),
        candidates: session.candidates().to_vec(),
        highlighted: session.highlighted() as u64,
    }
}

fn main() {
    let state = handle_in_memory_demo();

    // Touch the IPC request/response enums so the dependency is exercised and
    // the wire types stay in scope as the contract evolves.
    let _ = (Request::NewSession, Response::State(state.clone()));

    println!("ime-server M0 self-check");
    println!("  commit = {:?}", state.commit);
    println!("  COMMIT flag set = {}", state.flags & flags::COMMIT != 0);
    assert_eq!(state.commit, "ka");
    assert!(state.flags & flags::COMMIT != 0);
    println!("  OK");
}
