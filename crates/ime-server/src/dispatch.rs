//! Transport-agnostic request dispatch.
//!
//! The [`Dispatcher`] owns the engine and a registry of sessions keyed by id,
//! and turns an [`ime_ipc::Request`] into an [`ime_ipc::Response`]. Keeping this
//! independent of the named-pipe transport makes it unit-testable on the host
//! (the Windows pipe is just a byte stream feeding the same logic).

use ime_engine::{Engine, Key, Session};
use ime_ipc::{Request, Response, State};
use std::collections::HashMap;

pub struct Dispatcher {
    engine: Engine,
    sessions: HashMap<u64, Session>,
    next_id: u64,
}

impl Dispatcher {
    pub fn new(engine: Engine) -> Self {
        Dispatcher {
            engine,
            sessions: HashMap::new(),
            next_id: 1,
        }
    }

    /// Number of live sessions (for diagnostics/tests).
    #[allow(dead_code)]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn handle(&mut self, req: Request) -> Response {
        match req {
            Request::NewSession => {
                let sid = self.next_id;
                self.next_id += 1;
                self.sessions.insert(sid, self.engine.new_session());
                Response::SessionId { sid }
            }
            Request::CloseSession { sid } => {
                self.sessions.remove(&sid);
                Response::Ok
            }
            Request::ProcessKey { sid, keysym, mods } => {
                self.with_session(sid, |s| s.process_key(Key::new(keysym, mods)))
            }
            Request::SelectCandidate { sid, index } => {
                self.with_session(sid, |s| s.select_candidate(index as usize))
            }
            Request::Reset { sid } => self.with_session(sid, |s| s.reset()),
            // Cloud-AI conversion arrives in M2. Until then, report it as
            // unavailable so the frontend falls back to the local kana.
            Request::BeginAiConvert { .. } | Request::PollAiResult { .. } => Response::Error {
                message: "AI conversion not implemented (M2)".to_owned(),
            },
        }
    }

    /// Run `f` on the addressed session and return its resulting [`State`], or an
    /// error response if the session id is unknown.
    fn with_session<F>(&mut self, sid: u64, f: F) -> Response
    where
        F: FnOnce(&mut Session) -> u32,
    {
        match self.sessions.get_mut(&sid) {
            Some(session) => {
                let flags = f(session);
                Response::State(state_of(session, flags))
            }
            None => Response::Error {
                message: format!("unknown session {sid}"),
            },
        }
    }
}

fn state_of(session: &Session, flags: u32) -> State {
    State {
        flags,
        preedit: session.preedit().to_owned(),
        commit: session.commit_text().to_owned(),
        candidates: session.candidates().to_vec(),
        highlighted: session.highlighted() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_engine::{flags, keysym};

    fn new_dispatcher() -> Dispatcher {
        Dispatcher::new(Engine::new(None, None))
    }

    fn open_session(d: &mut Dispatcher) -> u64 {
        match d.handle(Request::NewSession) {
            Response::SessionId { sid } => sid,
            other => panic!("expected SessionId, got {other:?}"),
        }
    }

    #[test]
    fn new_session_returns_increasing_ids() {
        let mut d = new_dispatcher();
        assert_eq!(open_session(&mut d), 1);
        assert_eq!(open_session(&mut d), 2);
        assert_eq!(d.session_count(), 2);
    }

    #[test]
    fn process_key_converts_romaji() {
        let mut d = new_dispatcher();
        let sid = open_session(&mut d);
        for ch in "konnichiha".chars() {
            d.handle(Request::ProcessKey {
                sid,
                keysym: ch as u32,
                mods: 0,
            });
        }
        let resp = d.handle(Request::ProcessKey {
            sid,
            keysym: keysym::RETURN,
            mods: 0,
        });
        match resp {
            Response::State(state) => {
                assert!(state.flags & flags::COMMIT != 0);
                assert_eq!(state.commit, "こんにちは");
            }
            other => panic!("expected State, got {other:?}"),
        }
    }

    #[test]
    fn unknown_session_is_an_error() {
        let mut d = new_dispatcher();
        let resp = d.handle(Request::ProcessKey {
            sid: 999,
            keysym: 'a' as u32,
            mods: 0,
        });
        assert!(matches!(resp, Response::Error { .. }));
    }

    #[test]
    fn close_session_frees_it() {
        let mut d = new_dispatcher();
        let sid = open_session(&mut d);
        assert_eq!(d.session_count(), 1);
        assert!(matches!(
            d.handle(Request::CloseSession { sid }),
            Response::Ok
        ));
        assert_eq!(d.session_count(), 0);
    }

    #[test]
    fn ai_convert_reports_unavailable_in_m1() {
        let mut d = new_dispatcher();
        let sid = open_session(&mut d);
        let resp = d.handle(Request::BeginAiConvert {
            sid,
            context_before: String::new(),
            context_after: String::new(),
        });
        assert!(matches!(resp, Response::Error { .. }));
    }
}
