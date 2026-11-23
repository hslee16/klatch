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
//! with `X-Klatch-Member`), checked live; the certificate's subject is never read, mapped or logged.
//!
//! **Revocation: NO CRL and NO OCSP is checked.** A client certificate that was valid when it was issued stays accepted
//! until it expires or until the CA file is replaced (the file is read at start, so replacing it takes a restart).
//! Revocation therefore means short-lived certificates, or a new CA. A CRL option is ticket #1491.
//!
//! **The secret key.** The key file must be a regular file (not a symbolic link) owned by the user running the server and
//! mode 0600 (no bit for group or other), or the server refuses to start; it is opened without blocking, so a FIFO named
//! as the key is refused and cannot hold startup. Key bytes are read once, handed to rustls and dropped; no message, log
//! line or error holds any of them. They are not zeroised on drop: the process disables core dumps at start
//! (`harden` in `main.rs`), which is what keeps them out of a core file.
//!
//! **The handshake is bounded.** A short deadline, and a cap on connections in the handshake that closes the oldest to
//! admit a newcomer (`tls_limits.rs`), so silent sockets cannot keep a valid client out; a flood by volume is still a
//! denial of service for anyone who can reach the port.
//!
//! **How a connection is served.** The HTTP handlers (`wire_http.rs`) take a connected stream and nothing else, so after
//! the handshake this module makes an in-process pair of sockets, gives one end to the handler and carries the other
//! end's bytes through TLS (`tls_bridge::bridge`). Nothing listens for the handler: the pair has no address, so no local process
//! can reach the plaintext side. The pump is `tls_bridge.rs`.

use crate::loopback::{Stream, connected_pair};
use crate::tls_limits::{Handshakes, Limits};
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

/// Read a file the operator named; the error names the file and what kind of failure, never the content.
fn read_pem(path: &Path, what: &str) -> Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("cannot read the {what} {}: {}", path.display(), e.kind()))
}

/// The private key, read only after the file passed its checks: a regular file (opened without following a symbolic
/// link, and without blocking, so a FIFO cannot hold startup), owned by this user, with no access for group or other.
fn read_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let shown = path.display();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
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

/// The certificates the client CA file holds (possibly none: the bind policy refuses a listener with no root, in its own
/// words, before any server configuration is built).
pub fn client_roots(path: &Path) -> Result<RootCertStore> {
    let bytes = read_pem(path, "client CA file")?;
    let mut roots = RootCertStore::empty();
    for ca in CertificateDer::pem_slice_iter(&bytes) {
        let ca = ca.map_err(|_| {
            anyhow::anyhow!("the client CA file {} is not valid PEM", path.display())
        })?;
        roots.add(ca).map_err(|_| {
            anyhow::anyhow!(
                "the client CA file {} holds a certificate rustls cannot use",
                path.display()
            )
        })?;
    }
    Ok(roots)
}

/// The server configuration for these files and client roots, or the reason the server must not start. Client
/// authentication is mandatory and nothing in this function can turn it off.
pub fn build_config(files: &TlsFiles, roots: RootCertStore) -> Result<Arc<ServerConfig>> {
    let key = read_key(&files.key)?;
    let chain = read_certs(&files.cert, "server certificate")?;
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

/// Finish the handshake within `deadline`. An error is any way of not having a verified client.
fn handshake(
    conn: &mut ServerConnection,
    sock: &mut Stream,
    deadline: Duration,
) -> std::io::Result<()> {
    let until = Instant::now() + deadline;
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

/// What `serve_tls` needs besides the connection: the TLS configuration, the limits and the handshakes in progress.
pub struct Listener<'a> {
    pub config: &'a Arc<ServerConfig>,
    pub limits: &'a Limits,
    pub handshakes: &'a Handshakes,
}

/// Serve one accepted connection: the handshake (counted among the handshakes in progress, with a deadline), then
/// `inner` on one end of a pair of local sockets while the other end is carried through TLS. A connection that fails
/// the handshake is dropped without a byte of HTTP.
pub fn serve_tls(mut sock: Stream, at: &Listener, inner: impl Fn(Stream) + Sync) {
    let Ok(mut conn) = ServerConnection::new(at.config.clone()) else {
        return;
    };
    let Some(ticket) = at.handshakes.enter(&sock, at.limits.max_handshakes) else {
        return;
    };
    let shaken = handshake(&mut conn, &mut sock, at.limits.handshake);
    at.handshakes.leave(ticket);
    if shaken.is_err() {
        let _ = sock.shutdown(std::net::Shutdown::Both);
        return;
    }
    conn.set_buffer_limit(Some(crate::tls_bridge::WINDOW));
    let Ok((handler_end, pump_end)) = connected_pair() else {
        return;
    };
    std::thread::scope(|scope| {
        scope.spawn(|| inner(handler_end));
        crate::tls_bridge::bridge(conn, &mut sock, pump_end, at.limits.stall);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

    /// A throwaway CA certificate, PEM.
    fn ca_pem() -> String {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let key = KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().pem()
    }

    /// The configuration for a throwaway set of files in a temp directory (this test's own).
    fn config_for(dir: &str) -> (Arc<ServerConfig>, CertificateDer<'static>) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::path::PathBuf::from(format!("/tmp/kt-unit-{}-{dir}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        params.is_ca = IsCa::NoCa;
        let key = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        let files = TlsFiles {
            cert: dir.join("c.pem"),
            key: dir.join("k.pem"),
            client_ca: dir.join("ca.pem"),
        };
        std::fs::write(&files.cert, cert.pem()).unwrap();
        std::fs::write(&files.key, key.serialize_pem()).unwrap();
        std::fs::set_permissions(&files.key, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&files.client_ca, ca_pem()).unwrap();
        let roots = client_roots(&files.client_ca).unwrap();
        let config = build_config(&files, roots).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (config, cert.der().clone())
    }

    /// Run a handshake in memory (no socket, and not `handshake` above, so not its `check_peer`) between this server
    /// configuration and a client that presents no certificate, and say whether the SERVER refused it.
    fn server_refuses_a_client_without_a_certificate(
        config: &Arc<ServerConfig>,
        server_cert: CertificateDer<'static>,
    ) -> bool {
        let mut roots = RootCertStore::empty();
        roots.add(server_cert).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let client_config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(rustls::DEFAULT_VERSIONS)
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = rustls_pki_types::ServerName::try_from("localhost").unwrap();
        let mut client = rustls::ClientConnection::new(Arc::new(client_config), name).unwrap();
        let mut server = ServerConnection::new(config.clone()).unwrap();
        for _ in 0..16 {
            let mut to_server = Vec::new();
            while client.wants_write() {
                client.write_tls(&mut to_server).unwrap();
            }
            if let Err(e) = feed_server(&mut server, &to_server) {
                // the server's own refusal, not an alert the client sent because it distrusted the server
                return matches!(e, rustls::Error::NoCertificatesPresented);
            }
            let mut to_client = Vec::new();
            while server.wants_write() {
                server.write_tls(&mut to_client).unwrap();
            }
            let mut input = &to_client[..];
            while !input.is_empty() {
                client.read_tls(&mut input).unwrap();
                let _ = client.process_new_packets();
            }
            if !server.is_handshaking() {
                return false;
            }
        }
        false
    }

    /// Give the server what the client sent.
    fn feed_server(server: &mut ServerConnection, bytes: &[u8]) -> Result<(), rustls::Error> {
        let mut input = bytes;
        while !input.is_empty() {
            server.read_tls(&mut input).unwrap();
            server.process_new_packets()?;
        }
        Ok(())
    }

    #[test]
    fn the_server_configuration_refuses_a_client_without_a_certificate_by_itself() {
        // at the handshake, through rustls alone: without the second check of `handshake` (which this does not run) a
        // verifier that let an unauthenticated client through would complete the handshake here and fail this test
        let (config, cert) = config_for("verifier");
        assert!(
            server_refuses_a_client_without_a_certificate(&config, cert),
            "a client with no certificate completed the handshake: client authentication is optional"
        );
    }

    #[test]
    fn a_ca_file_with_no_certificate_gives_an_empty_root_store_and_is_not_an_error_here() {
        let path =
            std::path::PathBuf::from(format!("/tmp/kt-unit-{}-empty-ca", std::process::id()));
        std::fs::write(&path, "no certificate in here\n").unwrap();
        assert!(client_roots(&path).unwrap().is_empty());
        std::fs::write(&path, ca_pem()).unwrap();
        assert!(!client_roots(&path).unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
