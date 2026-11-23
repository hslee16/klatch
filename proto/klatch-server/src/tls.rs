//! The TLS half of the mutual TLS listener (#108, epic #13, decisions of #157 recorded 2026-11-23): rustls with the
//! operator's own files and nothing else. This module and `tls_bridge.rs` are the only places the crate names `rustls`
//! (`tests/no_network.rs` scans for it, and for every rustls call that would skip a check).
//!
//! **What the operator supplies.** A server certificate chain and its private key (PEM), and a CA file (PEM) holding the
//! certificates a client's certificate must chain to. klatch makes no certificate, runs no CA and enrols nobody.
//!
//! **What the handshake enforces.** The client verifier is rustls's `WebPkiClientVerifier` built on the CA file with
//! client authentication mandatory: a client without a certificate, with one from another CA, with an expired or
//! not-yet-valid one, or one that is not for client use, fails the handshake and gets no byte of HTTP. There is no
//! option to skip any of it (no `dangerous` configuration, no optional client authentication, no plaintext mode). The
//! protocol versions are rustls's defaults, TLS 1.3 and TLS 1.2 (no 1.1 or 1.0 exist in the library, so a downgrade has
//! nothing to fall back to); session resumption is off, so every connection passes the whole check again (an expired
//! certificate or a later CA file is never trusted from a cache).
//!
//! **What a certificate is.** A key to the port, not a name inside klatch. Once the handshake passed, the connection is
//! handed to the same HTTP handlers as the loopback listener, where the caller is still the worker credential (`Bearer`
//! with `X-Klatch-Member`), checked live; the certificate's subject is never read, mapped or logged. Revocation is the
//! operator's: short-lived certificates, or a new CA file (the CA file is read at start).
//!
//! **The secret key.** The key file must be a regular file (not a symbolic link) owned by the user running the server and
//! mode 0600 (no bit for group or other), or the server refuses to start. Key bytes are read once, handed to rustls and
//! dropped; no message, log line or error holds any of them.
//!
//! **How a connection is served.** The HTTP handlers (`wire_http.rs`) take a connected stream and nothing else, so after
//! the handshake this module makes an in-process pair of sockets, gives one end to the handler and carries the other
//! end's bytes through TLS (`tls_bridge::bridge`). Nothing listens for the handler: the pair has no address, so no local process
//! can reach the plaintext side. The pump is `tls_bridge.rs`.

use crate::loopback::{Stream, connected_pair};

use anyhow::{Context, Result, bail};
use klatch::session::policy::{Peer, Security, check_peer};
use rustls::server::{NoServerSessionStorage, WebPkiClientVerifier};
use rustls::{RootCertStore, ServerConfig, ServerConnection};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The files the operator names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub client_ca: PathBuf,
}

/// How long a client has to finish the handshake (a client that stalls there holds one connection slot this long).
const HANDSHAKE: Duration = Duration::from_secs(5);

/// Read a file the operator named; the error names the file and what kind of failure, never the content.
fn read_pem(path: &Path, what: &str) -> Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("cannot read the {what} {}: {}", path.display(), e.kind()))
}

/// The private key, read only after the file passed its checks: a regular file (opened without following a symbolic
/// link), owned by this user, with no access for group or other.
fn read_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let shown = path.display();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => {
                anyhow::anyhow!("the key file {shown} is a symbolic link: name the file itself")
            }
            _ => anyhow::anyhow!("cannot open the key file {shown}: {}", e.kind()),
        })?;
    // checked on the open file, so the file that was checked is the file that is read
    let meta = file.metadata().context("cannot inspect the key file")?;
    if !meta.is_file() {
        bail!("the key file {shown} is not a regular file");
    }
    // SAFETY: geteuid has no preconditions.
    if meta.uid() != unsafe { libc::geteuid() } {
        bail!("the key file {shown} is not owned by the user running the server");
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "the key file {shown} must be mode 0600 (it is {mode:04o}): only its owner may read a private key"
        );
    }
    let mut bytes = Vec::new();
    (&file)
        .read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("cannot read the key file {shown}: {}", e.kind()))?;
    PrivateKeyDer::from_pem_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("the key file {shown} holds no private key (PEM)"))
}

/// The certificates of a PEM file (at least one).
fn read_certs(path: &Path, what: &str) -> Result<Vec<CertificateDer<'static>>> {
    let bytes = read_pem(path, what)?;
    let certs: Vec<_> = CertificateDer::pem_slice_iter(&bytes)
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("the {what} {} is not valid PEM", path.display()))?;
    if certs.is_empty() {
        bail!("the {what} {} holds no certificate", path.display());
    }
    Ok(certs)
}

/// The server configuration for these files, or the reason the server must not start. Client authentication is
/// mandatory and nothing in this function can turn it off.
pub fn load_config(files: &TlsFiles) -> Result<Arc<ServerConfig>> {
    let key = read_key(&files.key)?;
    let chain = read_certs(&files.cert, "server certificate")?;
    let mut roots = RootCertStore::empty();
    for ca in read_certs(&files.client_ca, "client CA file")? {
        roots.add(ca).map_err(|_| {
            anyhow::anyhow!(
                "the client CA file {} holds a certificate rustls cannot use",
                files.client_ca.display()
            )
        })?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| anyhow::anyhow!("cannot build the client certificate check: {e}"))?;
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::DEFAULT_VERSIONS)
        .map_err(|e| anyhow::anyhow!("cannot set the TLS versions: {e}"))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(|_| {
            anyhow::anyhow!(
                "the server certificate and key do not fit together, or the certificate is unusable"
            )
        })?;
    // every connection passes the whole handshake: no cached session, no ticket
    config.session_storage = Arc::new(NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    Ok(Arc::new(config))
}

/// Finish the handshake within [`HANDSHAKE`]. An error is any way of not having a verified client.
fn handshake(conn: &mut ServerConnection, sock: &mut Stream) -> std::io::Result<()> {
    let until = Instant::now() + HANDSHAKE;
    while conn.is_handshaking() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        sock.set_read_timeout(Some(left))?;
        sock.set_write_timeout(Some(left))?;
        conn.complete_io(sock)?;
    }
    // what the handshake proved is checked once more against the policy, so a verifier that let a peer through without a
    // chain would still stop here
    let chain_verified = conn.peer_certificates().is_some_and(|c| !c.is_empty());
    check_peer(Security::MutualTls, &Peer::Certificate { chain_verified }).map_err(|_| {
        std::io::Error::new(
            ErrorKind::PermissionDenied,
            "no verified client certificate",
        )
    })
}

/// Serve one accepted connection: the handshake, then `inner` on one end of a pair of local sockets while the other end
/// is carried through TLS. A connection that fails the handshake is dropped without a byte of HTTP.
pub fn serve_tls(mut sock: Stream, config: &Arc<ServerConfig>, inner: impl Fn(Stream) + Sync) {
    let Ok(mut conn) = ServerConnection::new(config.clone()) else {
        return;
    };
    if handshake(&mut conn, &mut sock).is_err() {
        let _ = sock.shutdown(std::net::Shutdown::Both);
        return;
    }
    conn.set_buffer_limit(Some(crate::tls_bridge::WINDOW));
    let Ok((handler_end, pump_end)) = connected_pair() else {
        return;
    };
    std::thread::scope(|scope| {
        scope.spawn(|| inner(handler_end));
        crate::tls_bridge::bridge(conn, &mut sock, pump_end);
    });
}
