//! The Rust client over the klatch-server daemon's Unix socket (#1095, feature `client-socket`). This is the only file of
//! the `klatch` crate that names a socket type, and only `UnixStream`: it connects to a path on this machine and listens
//! for nothing. The feature is in no default feature (`cli`, `client` and `default` never turn it on), so the shipped
//! `klatch` binary does not contain this code; `tests/no_telemetry.rs` pins both facts.

use super::{Client, Error, Identity, Result};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

impl Client {
    /// Connect to a `klatch-server daemon --socket PATH` and say who you are (`identity`, the `identify` first
    /// request). The hello is read first, as over stdio; `member` and `attribution` are then what the daemon
    /// answered. A refused identity is an `Error::Api` with code `identity.refused` (the message never repeats the
    /// name or the token); a path nobody listens on is an `Error::Session`. [`Client::close`] shuts the socket and
    /// waits for the daemon to end the session; it answers 0.
    pub fn connect_socket(path: &Path, identity: &Identity) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .map_err(|e| Error::Session(format!("could not connect to {}: {e}", path.display())))?;
        let dup = |s: &UnixStream| {
            s.try_clone()
                .map_err(|e| Error::Session(format!("could not use the socket: {e}")))
        };
        let (reader, writer) = (dup(&stream)?, dup(&stream)?);
        let shutdown = Box::new(move || {
            let _ = stream.shutdown(Shutdown::Write);
        });
        Self::over(reader, writer, shutdown, identity)
    }
}
