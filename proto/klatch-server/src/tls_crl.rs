//! The client certificate revocation list of the mutual TLS listener (#1491, refs #108, #157): `--tls-client-crl FILE`.
//! Names rustls types, so it is one of the TLS modules `tests/no_network.rs` scans.
//!
//! **What the operator supplies.** One PEM file holding one or more X.509 CRLs, signed by the CA (or CAs) of the client CA
//! file. The file is read once at start; replacing it takes a restart (reload without one is #1492).
//!
//! **What starts the server.** The file must be a regular file (not a symbolic link, opened without blocking), not
//! writable by group or other (whoever can write the file can un-revoke a certificate), not empty, valid PEM, and hold at
//! least one CRL, or the server refuses to start in words that name the file. It never prints CRL content.
//!
//! **Policy** (rustls `WebPkiClientVerifier`, set in `tls.rs`):
//! - A client certificate whose serial is listed by the CRL of its issuer fails the handshake: no session, no HTTP.
//! - Expiry is ENFORCED (`enforce_revocation_expiration`): once a CRL's `nextUpdate` has passed, every client certificate
//!   its issuer signed is refused, because a stale list proves nothing. The operator is told at start (a log line) and at
//!   every refusal (a log line, `note_refusal`, at most one per kind every ten seconds), and the fix is a fresh CRL file
//!   and a restart. The alternative, ignoring expiry, would keep a revoked certificate working for as long as the
//!   operator forgot to renew the list.
//! - Unknown status is ALLOWED (`UnknownStatusPolicy::Allow`): a certificate that no configured CRL covers (another
//!   issuer, or an intermediate CA with no list from its parent) is accepted exactly as it is without this option. The
//!   rustls alternative, deny, would refuse every certificate whose issuer has no CRL, including the intermediate CAs of a
//!   chain, which locks out an operator who lists only the CRL of the CA that issues clients. The cost: a CRL signed by a
//!   CA that issued none of the clients does nothing, and nothing here can tell (a start-up check is ticket #1614).
//! - The whole chain is checked, the trust anchor excepted (rustls's default).

use anyhow::{Result, bail};
use rustls_pki_types::CertificateRevocationListDer;
use rustls_pki_types::pem::PemObject;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The CRLs of the file, after its checks (see the module doc). The error names the file and the failure, never content.
pub fn read_crls(path: &Path) -> Result<Vec<CertificateRevocationListDer<'static>>> {
    let shown = path.display();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => {
                anyhow::anyhow!(
                    "the client CRL file {shown} is a symbolic link: name the file itself"
                )
            }
            _ => anyhow::anyhow!("cannot open the client CRL file {shown}: {}", e.kind()),
        })?;
    // checked on the open file, so the file that was checked is the file that is read
    let meta = file
        .metadata()
        .map_err(|e| anyhow::anyhow!("cannot inspect the client CRL file {shown}: {}", e.kind()))?;
    if !meta.is_file() {
        bail!("the client CRL file {shown} is not a regular file");
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o022 != 0 {
        bail!(
            "the client CRL file {shown} is writable by group or other (mode {mode:04o}): whoever can write it can un-revoke a certificate; use 0644 or stricter"
        );
    }
    let mut bytes = Vec::new();
    (&file)
        .read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("cannot read the client CRL file {shown}: {}", e.kind()))?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        bail!("the client CRL file {shown} is empty");
    }
    let crls: Vec<_> = CertificateRevocationListDer::pem_slice_iter(&bytes)
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("the client CRL file {shown} is not valid PEM"))?;
    if crls.is_empty() {
        bail!("the client CRL file {shown} holds no CRL (a PEM block X509 CRL)");
    }
    Ok(crls)
}

/// The line printed at start when a CRL is in force.
pub fn start_line(path: &Path) -> String {
    format!(
        "klatch-server: https: client certificate revocation: CRL {} in force (read at start; replace it and restart to change it; expiry is enforced: when a CRL passes its nextUpdate, every client certificate of its issuer is refused until you replace the CRL file and restart; a certificate no listed CRL covers is allowed)",
        path.display()
    )
}

/// One log line per kind at most this often, so a client that retries a revoked certificate cannot flood the log.
const EVERY: Duration = Duration::from_secs(10);

static LAST: [Mutex<Option<Instant>>; 3] = [const { Mutex::new(None) }; 3];

/// Say, in the server log, why a handshake failed when the cause is the revocation check (and say nothing for any other
/// failure: those are the clients that never had a certificate, and the operator did not ask to hear of each).
pub fn note_refusal(err: &std::io::Error) {
    use rustls::CertificateError as C;
    let Some(rustls::Error::InvalidCertificate(why)) = err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    else {
        return;
    };
    let (slot, text) = match why {
        C::Revoked => (0, "revoked (listed by the client CRL)"),
        C::ExpiredRevocationList | C::ExpiredRevocationListContext { .. } => (
            1,
            "the CRL has expired (its nextUpdate has passed), so every client certificate of its issuer is refused: replace the CRL file and restart the server (reload without a restart is #1492)",
        ),
        C::UnknownRevocationStatus => (2, "its revocation status is unknown"),
        _ => return,
    };
    let mut last = LAST[slot].lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| t.elapsed() < EVERY) {
        return;
    }
    *last = Some(Instant::now());
    eprintln!("klatch-server: https: client certificate refused: {text}");
}
