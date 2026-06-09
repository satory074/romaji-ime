//! `ime-server`: hosts the IME engine out-of-process for the Windows frontend.
//!
//! The thin TSF DLL (loaded into every app) forwards key events here over a
//! named pipe; this process owns the engine and does all the heavy work
//! (dictionary, Viterbi, and the slow cloud-LLM round-trip) off the apps'
//! threads, giving both crash isolation and a non-blocking input path.
//!
//! The request/response logic lives in [`dispatch`] (host-testable); the named
//! pipe (`pipe_win`, Windows only) merely supplies a byte stream to
//! [`transport::serve_connection`].

mod dispatch;
#[cfg(windows)]
mod pipe_win;
mod transport;

use dispatch::Dispatcher;
use ime_engine::Engine;

/// The per-user named pipe. M4 will randomize/secure this name to prevent
/// squatting (as Mozc/Weasel do).
#[cfg(windows)]
const PIPE_NAME: &str = r"\\.\pipe\romaji_ime";

#[cfg(windows)]
fn main() -> std::io::Result<()> {
    // Cloud-AI converter from config.json / env (the headline feature).
    let engine = Engine::new(None, None).with_ai_from_config();
    let mut dispatcher = Dispatcher::new(engine);
    eprintln!("ime-server listening on {PIPE_NAME}");
    pipe_win::run(PIPE_NAME, &mut dispatcher)
}

/// Off-Windows there is no named pipe; run a dispatcher self-check so
/// `cargo run -p ime-server` still demonstrates the engine wiring on the dev
/// host (and `cargo check --target x86_64-pc-windows-msvc` validates the
/// Windows path separately).
#[cfg(not(windows))]
fn main() {
    use ime_engine::{flags, keysym};
    use ime_ipc::{Request, Response};

    let mut dispatcher = Dispatcher::new(Engine::new(None, None).with_ai_from_config());
    let sid = match dispatcher.handle(Request::NewSession) {
        Response::SessionId { sid } => sid,
        other => panic!("expected SessionId, got {other:?}"),
    };
    for ch in "ka".chars() {
        dispatcher.handle(Request::ProcessKey {
            sid,
            keysym: ch as u32,
            mods: 0,
        });
    }
    let resp = dispatcher.handle(Request::ProcessKey {
        sid,
        keysym: keysym::RETURN,
        mods: 0,
    });

    println!("ime-server self-check (dispatcher, non-Windows host)");
    match resp {
        Response::State(state) => {
            println!("  commit = {:?}", state.commit);
            assert_eq!(state.commit, "か");
            assert!(state.flags & flags::COMMIT != 0);
            println!("  OK");
        }
        other => panic!("expected State, got {other:?}"),
    }
}
