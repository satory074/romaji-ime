//! Windows named-pipe transport for `ime-server`.
//!
//! Compiled only on Windows. Wraps a connected pipe instance as a `Read + Write`
//! stream and hands it to [`crate::transport::serve_connection`].
//!
//! M1: serves clients **sequentially** (one app at a time). Concurrent apps and
//! a randomized/secured pipe name are an M4 hardening item. This file is
//! validated on the dev host via `cargo check --target x86_64-pc-windows-msvc`.

use crate::dispatch::Dispatcher;
use crate::transport::serve_connection;
use std::io::{self, Read, Write};
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
};
// PIPE_ACCESS_* are FILE_FLAGS_AND_ATTRIBUTES (the dwOpenMode arg), so they live
// in Storage::FileSystem alongside ReadFile/WriteFile.
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

type Handle = windows_sys::Win32::Foundation::HANDLE;

const BUFFER_SIZE: u32 = 64 * 1024;

/// A connected named-pipe instance, viewed as a byte stream.
struct PipeStream {
    handle: Handle,
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            // A disconnecting client surfaces as a broken pipe -> treat as EOF.
            if err == ERROR_BROKEN_PIPE {
                return Ok(0);
            }
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        Ok(read as usize)
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                self.handle,
                buf.as_ptr().cast(),
                buf.len() as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32
            ));
        }
        Ok(written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn to_wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Serve clients on `pipe_name` forever (blocks). Each client gets a fresh pipe
/// instance; sessions live in the shared [`Dispatcher`].
pub fn run(pipe_name: &str, dispatcher: &mut Dispatcher) -> io::Result<()> {
    let wide = to_wide_nul(pipe_name);
    loop {
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        // Block until a client connects. ERROR_PIPE_CONNECTED means a client
        // connected between Create and Connect, which is still success.
        let connected = unsafe { ConnectNamedPipe(handle, ptr::null_mut()) };
        let ok = connected != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if ok {
            let mut stream = PipeStream { handle };
            let _ = serve_connection(dispatcher, &mut stream);
            unsafe {
                DisconnectNamedPipe(handle);
            }
        }
        unsafe {
            CloseHandle(handle);
        }
    }
}
