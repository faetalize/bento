//! Blocking client for bento's socket.

use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::{Event, Request, Response};

/// Name of the environment variable that holds the socket's path.
pub const SOCKET_PATH_ENV: &str = "BENTO_SOCKET";

/// A connection to bento's socket.
pub struct Socket {
    stream: BufReader<UnixStream>,
}

impl Socket {
    /// Connects to the socket at the path in [`SOCKET_PATH_ENV`].
    pub fn connect() -> io::Result<Self> {
        let path = env::var_os(SOCKET_PATH_ENV).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{SOCKET_PATH_ENV} is not set, are you running this within bento?"),
            )
        })?;
        Self::connect_to(path)
    }

    /// Connects to the socket at `path`.
    pub fn connect_to(path: impl AsRef<Path>) -> io::Result<Self> {
        let stream = UnixStream::connect(path.as_ref())?;
        Ok(Self {
            stream: BufReader::new(stream),
        })
    }

    /// Sends a request and returns bento's answer.
    ///
    /// Return values:
    ///
    /// * `Ok(Ok(response))`: bento's [`Response`]
    /// * `Ok(Err(message))`: an error message from bento
    /// * `Err(error)`: an error talking to bento
    pub fn send(&mut self, request: &Request) -> io::Result<Result<Response, String>> {
        let line = serde_json::to_string(request).map_err(io::Error::other)?;
        let reply = self.send_line(&line)?;
        let response: Response = serde_json::from_str(&reply)?;
        Ok(response.into_result())
    }

    /// Sends one line of JSON as it is and returns the line bento answers with.
    pub fn send_line(&mut self, line: &str) -> io::Result<String> {
        let mut buf = String::from(line.trim_end());
        buf.push('\n');
        self.stream.get_mut().write_all(buf.as_bytes())?;
        self.read_line()
    }

    /// Reads the next line bento sends, without its line break. This blocks until there is one.
    pub fn read_line(&mut self) -> io::Result<String> {
        let mut buf = String::new();
        if self.stream.read_line(&mut buf)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf.truncate(buf.trim_end().len());
        Ok(buf)
    }

    /// Starts reading the [`Event`]s of a stream.
    ///
    /// The returned function blocks until the next event arrives. Use this only after a
    /// [`Request::Subscribe`] that bento answered.
    pub fn read_events(self) -> impl FnMut() -> io::Result<Event> {
        let Self { mut stream } = self;
        let _ = stream.get_mut().shutdown(Shutdown::Write);

        move || {
            let mut buf = String::new();
            if stream.read_line(&mut buf)? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            Ok(serde_json::from_str(&buf)?)
        }
    }
}
