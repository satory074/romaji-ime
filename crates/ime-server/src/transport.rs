//! Stream transport: drive a [`Dispatcher`] over any `Read + Write`.
//!
//! The Windows named pipe (see `pipe_win`) wraps a pipe handle as a stream and
//! hands it here, so the request loop itself is platform-independent and
//! host-testable.

use crate::dispatch::Dispatcher;
use ime_ipc::{read_frame, write_frame, Request};
use std::io::{self, Read, Write};

/// Serve one connected client until it disconnects (clean EOF) or errors.
#[allow(dead_code)] // used by the Windows pipe transport and by tests
pub fn serve_connection<S: Read + Write>(
    dispatcher: &mut Dispatcher,
    stream: &mut S,
) -> io::Result<()> {
    loop {
        let req: Request = match read_frame(stream) {
            Ok(req) => req,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let resp = dispatcher.handle(req);
        write_frame(stream, &resp)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_engine::keysym;
    use ime_ipc::{Request, Response};
    use std::io::Cursor;

    /// A fake duplex stream: reads drain `input`, writes append to `output`.
    struct MockStream {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }
    impl Write for MockStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serves_a_framed_request_stream() {
        use ime_engine::Engine;

        // Prepare a request stream: NewSession, then type "ka", then Return.
        let mut input = Vec::new();
        write_frame(&mut input, &Request::NewSession).unwrap();
        // Session id will be 1 (first session).
        write_frame(
            &mut input,
            &Request::ProcessKey {
                sid: 1,
                keysym: 'k' as u32,
                mods: 0,
            },
        )
        .unwrap();
        write_frame(
            &mut input,
            &Request::ProcessKey {
                sid: 1,
                keysym: 'a' as u32,
                mods: 0,
            },
        )
        .unwrap();
        write_frame(
            &mut input,
            &Request::ProcessKey {
                sid: 1,
                keysym: keysym::RETURN,
                mods: 0,
            },
        )
        .unwrap();

        let mut stream = MockStream {
            input: Cursor::new(input),
            output: Vec::new(),
        };
        let mut dispatcher = Dispatcher::new(Engine::new(None, None));
        serve_connection(&mut dispatcher, &mut stream).unwrap();

        // Parse the four responses back out of the output.
        let mut out = Cursor::new(stream.output);
        let r1: Response = read_frame(&mut out).unwrap();
        assert!(matches!(r1, Response::SessionId { sid: 1 }));
        let _r2: Response = read_frame(&mut out).unwrap(); // k -> preedit
        let _r3: Response = read_frame(&mut out).unwrap(); // a -> preedit か
        let r4: Response = read_frame(&mut out).unwrap();
        match r4 {
            Response::State(state) => assert_eq!(state.commit, "か"),
            other => panic!("expected State, got {other:?}"),
        }
    }
}
