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
//! **Revocation: a CRL if the operator gives one, never OCSP.** Without `--tls-client-crl` a client certificate that was
//! valid when it was issued stays accepted until it expires or until the CA file is replaced (the files are read at
//! start and again on SIGHUP, `tls_reload.rs`, #1492); revocation then means short-lived certificates, or a new CA. With it
//! (#1491) the one client verifier below also checks the listed CRLs: a listed serial fails the handshake, an expired CRL
//! is enforced (it refuses its issuer's certificates) and a certificate no CRL covers is allowed. The file, the checks
//! on it and the reasons for each choice are in `tls_crl.rs`. There is one verifier and one listener: the HTTP routes,
//! the event stream, MCP and the WebSocket route all sit behind this handshake. OCSP is not checked and nothing here
//! makes a network request.
//!
//! **The secret key.** The key file must be a regular file (not a symbolic link) owned by the user running the server and
//! mode 0600 (no bit for group or other), or the server refuses to start; it is opened without blocking, so a FIFO named
//! as the key is refused and cannot hold startup. Key bytes are read once, handed to rustls and dropped; no message, log
//! line or error holds any of them. They are not zeroised on drop: the process disables core dumps at start
//! (`harden` in `main.rs`), which is what keeps them out of a core file.
//!
//! **The handshake is bounded.** A short deadline, and a cap on connections in the handshake (`tls_limits.rs`): when the
//! cap is full a newcomer is refused if every handshake is younger than half the deadline, and otherwise takes the place of
//! the oldest. A flood of silent sockets is bounded by this, not removed: a flood by volume is still a denial of service
//! for anyone who can reach the port, and `tls_limits.rs` says which side loses in each case.
//!
//! **How a connection is served.** The HTTP handlers (`wire_http.rs`) take a connected stream and nothing else, so after
//! the handshake this module makes an in-process pair of sockets, gives one end to the handler and carries the other
//! end's bytes through TLS (`tls_bridge::bridge`). Nothing listens for the handler: the pair has no address, so no local process
//! can reach the plaintext side. The pump is `tls_bridge.rs`.

use crate::loopback::{Stream, connected_pair};
use crate::tls_admit::SessionSlots;
use crate::tls_crl;
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

/// A finished server configuration, shareable between connections (the name other modules use, so they never name rustls).
pub type SharedConfig = Arc<ServerConfig>;

/// The configuration type itself, for generic holders (`tls_reload::Current`).
pub type Config = ServerConfig;

/// The files the operator names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub client_ca: PathBuf,
    /// CRLs for client certificates (`--tls-client-crl`, #1491); `None` checks no revocation list.
    pub client_crl: Option<PathBuf>,
}

/// The most a certificate or CA file may hold: 4 MiB. A chain of certificates is a few KiB; more is a mistake.
const MAX_PEM_BYTES: u64 = 4 * 1024 * 1024;

/// Read a file the operator named, the way the key and CRL files are read: opened without following a link and without
/// blocking (so a FIFO cannot hold start or a reload), checked on the open descriptor to be a regular file of at most
/// 4 MiB, and, for a `trusted` file (the CA file: a trust anchor), owned by this user or root and not writable by group
/// or other (whoever can write it chooses who may connect). The error names the file and the kind of failure, never the
/// content.
fn read_pem(path: &Path, what: &str, trusted: bool) -> Result<Vec<u8>> {
    let shown = path.display();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => {
                anyhow::anyhow!("the {what} {shown} is a symbolic link: name the file itself")
            }
            _ => anyhow::anyhow!("cannot read the {what} {shown}: {}", e.kind()),
        })?;
    let meta = file
        .metadata()
        .map_err(|e| anyhow::anyhow!("cannot inspect the {what} {shown}: {}", e.kind()))?;
    if !meta.is_file() {
        bail!("the {what} {shown} is not a regular file");
    }
    if trusted {
        if meta.uid() != euid() && meta.uid() != 0 {
            bail!("the {what} {shown} is not owned by the user running the server or by root");
        }
        let mode = meta.mode() & 0o777;
        if mode & 0o022 != 0 {
            bail!(
                "the {what} {shown} is writable by group or other (mode {mode:04o}): whoever can write it chooses who may connect; use 0644 or stricter"
            );
        }
    }
    if meta.len() > MAX_PEM_BYTES {
        bail!("the {what} {shown} is larger than 4 MiB: it is not read");
    }
    let mut bytes = Vec::new();
    // bounded again at the read: a file that grew since it was inspected is cut off and refused
    (&file)
        .take(MAX_PEM_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("cannot read the {what} {shown}: {}", e.kind()))?;
    if bytes.len() as u64 > MAX_PEM_BYTES {
        bail!("the {what} {shown} grew beyond 4 MiB while it was being read: it is refused");
    }
    Ok(bytes)
}

/// The user the server runs as: the one place this crate asks (the key file's and the CRL file's owner checks use it).
pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
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
    if meta.uid() != euid() {
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
    let bytes = read_pem(path, what, false)?;
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
    let bytes = read_pem(path, "client CA file", true)?;
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

/// The server configuration for these files and client roots at a stage (start, or a reload that refuses a CRL file older
/// than the one in force), or the reason the server must not start; with the earliest `nextUpdate` of the CRLs it adopted
/// (`None` without a CRL file): what the next reload compares against. Client authentication is mandatory and nothing in
/// this function can turn it off.
pub fn build_config_at(
    files: &TlsFiles,
    roots: RootCertStore,
    stage: tls_crl::Stage,
) -> Result<(Arc<ServerConfig>, Option<i64>)> {
    let mut due = None;
    let key = read_key(&files.key)?;
    let chain = read_certs(&files.cert, "server certificate")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots_subjects: Vec<Vec<u8>> = roots.roots.iter().map(|a| a.subject.to_vec()).collect();
    let mut builder =
        WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone());
    if let Some(path) = &files.client_crl {
        // the CA subjects a CRL's issuer must match (#1614): taken from the roots the verifier was built on
        let subjects: Vec<&[u8]> = roots_subjects.iter().map(|s| s.as_ref()).collect();
        // expiry enforced, unknown status allowed: the reasons are in `tls_crl.rs`
        let (crls, earliest) = tls_crl::read_crls_at(path, &subjects, tls_crl::now_unix()?, stage)?;
        due = earliest;
        builder = builder
            .with_crls(crls)
            .allow_unknown_revocation_status()
            .enforce_revocation_expiration();
    }
    let verifier = builder.build().map_err(|e| match &files.client_crl {
        Some(path) => anyhow::anyhow!(
            "cannot build the client certificate check from the client CRL file {}: {e}",
            path.display()
        ),
        None => anyhow::anyhow!("cannot build the client certificate check: {e}"),
    })?;
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
    Ok((Arc::new(config), due))
}

/// One step of the handshake: send what the connection has to say, or read once and process it. `complete_io` would loop
/// on reads inside one call with the timeout set before it, so a peer that sends one byte at a time, each inside that
/// timeout, would never be cut; one read per step lets the caller shrink the timeout to the time left every time.
fn step(conn: &mut ServerConnection, sock: &mut Stream) -> std::io::Result<()> {
    if conn.wants_write() {
        conn.write_tls(sock)?;
        return Ok(());
    }
    if conn.wants_read() {
        if conn.read_tls(sock)? == 0 {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        if let Err(e) = conn.process_new_packets() {
            // a last try to send the alert that says why, which must not hide the error itself
            let _ = conn.write_tls(sock);
            return Err(std::io::Error::new(ErrorKind::InvalidData, e));
        }
    }
    Ok(())
}

/// Finish the handshake within `deadline`. An error is any way of not having a verified client.
fn handshake(
    conn: &mut ServerConnection,
    sock: &mut Stream,
    deadline: Duration,
) -> std::io::Result<()> {
    // a deadline that does not fit in a clock is not a deadline: the options are bounded, and this holds anyway
    let Some(until) = Instant::now().checked_add(deadline) else {
        return Err(ErrorKind::InvalidInput.into());
    };
    while conn.is_handshaking() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        sock.set_read_timeout(Some(left))?;
        sock.set_write_timeout(Some(left))?;
        step(conn, sock)?;
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
    /// The places of connections that passed the handshake (`--max-connections`), apart from the handshake pool.
    pub sessions: &'a SessionSlots,
}

/// Serve one accepted connection: the handshake (counted among the handshakes in progress, with a deadline), then
/// `inner` on one end of a pair of local sockets while the other end is carried through TLS. A connection that fails
/// the handshake is dropped without a byte of HTTP.
pub fn serve_tls(mut sock: Stream, at: &Listener, inner: impl Fn(Stream) + Sync) {
    let Ok(mut conn) = ServerConnection::new(at.config.clone()) else {
        return;
    };
    // refused when the cap is full of handshakes too young to close (`tls_limits.rs`): the connection is just dropped
    let Some(ticket) = at.handshakes.enter(
        &sock,
        at.limits.max_handshakes,
        at.limits.per_source_within(),
        at.limits.min_evict_age(),
    ) else {
        return;
    };
    let shaken = handshake(&mut conn, &mut sock, at.limits.handshake);
    drop(ticket);
    if let Err(e) = &shaken {
        tls_crl::note_refusal(e);
        let _ = sock.shutdown(std::net::Shutdown::Both);
        return;
    }
    // a place among the sessions, taken only now: a connection that never finished the handshake never held one. Over the
    // cap the connection is closed, as an excess connection always was.
    let Some(_session) = at.sessions.claim() else {
        let _ = sock.shutdown(std::net::Shutdown::Both);
        return;
    };
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
            client_crl: None,
        };
        std::fs::write(&files.cert, cert.pem()).unwrap();
        std::fs::write(&files.key, key.serialize_pem()).unwrap();
        std::fs::set_permissions(&files.key, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&files.client_ca, ca_pem()).unwrap();
        let roots = client_roots(&files.client_ca).unwrap();
        let config = build_config_at(&files, roots, tls_crl::Stage::Start)
            .unwrap()
            .0;
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
