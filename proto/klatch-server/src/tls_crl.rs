//! The client certificate revocation list of the mutual TLS listener (#1491, refs #108, #157): `--tls-client-crl FILE`.
//! Names rustls types, so it is one of the TLS modules `tests/no_network.rs` scans.
//!
//! **What the operator supplies.** One PEM file holding one or more X.509 CRLs, signed by the CA (or CAs) of the client CA
//! file. The file is read once at start; replacing it takes a restart (reload without one is #1492).
//!
//! **What starts the server.** The file must be a regular file (not a symbolic link, opened without blocking), owned by
//! the user running the server or by root, not writable by group or other (whoever can write the file can un-revoke a
//! certificate), at most 4 MiB (checked before it is read), not empty, valid PEM, hold at least one and at most 16 CRLs,
//! and no two CRLs of one issuer, or the server refuses to start in words that name the file. It never prints CRL
//! content.
//!
//! **One CRL per issuer.** rustls-webpki uses the FIRST CRL it finds for an issuer. `cat new.crl >> client.crl` would
//! therefore keep an older (even empty) list in force and ignore the new revocations, or keep an expired one in force and
//! refuse every client. To make that mistake impossible the server reads the issuer name of each CRL (`issuer_of`, a
//! small bounded DER reader: definite lengths only, no recursion, nothing allocated) and refuses a file in which two CRLs
//! share one: replace the file, never append.
//!
//! **Policy** (rustls `WebPkiClientVerifier`, set in `tls.rs`):
//! - A client certificate whose serial is listed by the CRL of its issuer fails the handshake: no session, no HTTP.
//! - Expiry is ENFORCED (`enforce_revocation_expiration`): once a CRL's `nextUpdate` has passed, every client certificate
//!   its issuer signed is refused, because a stale list proves nothing. The operator is told at start (a log line) and at
//!   every refusal (a log line, `note_refusal`, at most one per kind every ten seconds), and the fix is a fresh CRL file
//!   and a restart. A CRL the verifier cannot trust (a bad signature, an unsupported signature algorithm, an issuer
//!   certificate that may not sign CRLs) refuses every certificate of its issuer too, and says so in its own log line. The alternative, ignoring expiry, would keep a revoked certificate working for as long as the
//!   operator forgot to renew the list.
//! - Unknown status is ALLOWED (`UnknownStatusPolicy::Allow`): a certificate that no configured CRL covers (another
//!   issuer, or an intermediate CA with no list from its parent) is accepted exactly as it is without this option. The
//!   rustls alternative, deny, would refuse every certificate whose issuer has no CRL, including the intermediate CAs of a
//!   chain, which locks out an operator who lists only the CRL of the CA that issues clients. The cost: a CRL signed by a
//!   CA that issued none of the clients does nothing, and nothing here can tell (a start-up check is ticket #1614; the
//!   issuer name of each CRL, read here for the duplicate check, is what that check needs).
//! - The whole chain is checked, the trust anchor excepted (rustls's default).

use anyhow::{Result, bail};
use rustls_pki_types::CertificateRevocationListDer;
use rustls_pki_types::pem::PemObject;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The largest CRL file: a CRL of a CA with a very large number of revocations is a few MiB; more is a mistake.
const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The most CRLs one file may hold (one per issuer, and an operator has a handful of CAs).
const MAX_CRLS: usize = 16;

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
    // SAFETY: geteuid has no preconditions.
    if !owner_ok(meta.uid(), unsafe { libc::geteuid() }) {
        bail!("the client CRL file {shown} is not owned by the user running the server or by root");
    }
    if meta.len() > MAX_BYTES {
        bail!("the client CRL file {shown} is larger than 4 MiB: it is not read");
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o022 != 0 {
        bail!(
            "the client CRL file {shown} is writable by group or other (mode {mode:04o}): whoever can write it can un-revoke a certificate; use 0644 or stricter"
        );
    }
    let mut bytes = Vec::new();
    // bounded again at the read: a file that grew since it was inspected is cut off and refused, not read whole
    (&file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("cannot read the client CRL file {shown}: {}", e.kind()))?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("the client CRL file {shown} is larger than 4 MiB: it is not read");
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        bail!("the client CRL file {shown} is empty");
    }
    let mut crls: Vec<CertificateRevocationListDer<'static>> = Vec::new();
    for crl in CertificateRevocationListDer::pem_slice_iter(&bytes) {
        let crl =
            crl.map_err(|_| anyhow::anyhow!("the client CRL file {shown} is not valid PEM"))?;
        if crls.len() == MAX_CRLS {
            bail!(
                "the client CRL file {shown} holds more than {MAX_CRLS} CRLs: at most {MAX_CRLS} are accepted"
            );
        }
        crls.push(crl);
    }
    if crls.is_empty() {
        bail!("the client CRL file {shown} holds no CRL (a PEM block X509 CRL)");
    }
    check_one_per_issuer(&crls, path)?;
    Ok(crls)
}

/// Whether a file owned by `uid` may be trusted by a server running as `euid`: its own user, or root.
fn owner_ok(uid: u32, euid: u32) -> bool {
    uid == euid || uid == 0
}

/// Refuse two CRLs of one issuer (see the module doc), and a CRL whose issuer cannot be read.
fn check_one_per_issuer(crls: &[CertificateRevocationListDer<'_>], path: &Path) -> Result<()> {
    let shown = path.display();
    let mut seen: Vec<&[u8]> = Vec::new();
    for crl in crls {
        let Some(issuer) = issuer_of(crl.as_ref()) else {
            bail!(
                "the client CRL file {shown} holds a CRL whose issuer cannot be read: it is not a valid X.509 CRL"
            );
        };
        if seen.contains(&issuer) {
            bail!(
                "the client CRL file {shown} holds two CRLs of one issuer: one CRL per issuer, because only the first would be used and a newer list after an older one would be ignored; replace the file, never append"
            );
        }
        seen.push(issuer);
    }
    Ok(())
}

/// One DER element: its tag, its content and what follows. Definite, minimal lengths of at most four bytes only; a high
/// tag number, an indefinite length, a length that runs past the input and a length not in its shortest form are all
/// `None`. Nothing recurses and nothing allocates, so no input makes this slow or large.
fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n || rest[0] == 0 {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |a, b| (a << 8) | usize::from(*b));
        if len < 0x80 {
            return None;
        }
        (len, &rest[n..])
    };
    (len <= rest.len()).then(|| (tag, &rest[..len], &rest[len..]))
}

/// The issuer name (its DER content) of a CRL: `CertificateList ::= SEQUENCE { tbsCertList SEQUENCE { version INTEGER
/// OPTIONAL, signature AlgorithmIdentifier, issuer Name, ... }, ... }`. rustls-webpki compares issuers as these bytes.
fn issuer_of(crl: &[u8]) -> Option<&[u8]> {
    let (list_tag, list, after) = tlv(crl)?;
    let (tbs_tag, tbs, _) = tlv(list)?;
    if list_tag != 0x30 || tbs_tag != 0x30 || !after.is_empty() {
        return None;
    }
    let (mut tag, _, mut rest) = tlv(tbs)?;
    if tag == 0x02 {
        // the optional version; the signature algorithm is the element after it
        (tag, _, rest) = tlv(rest)?;
    }
    if tag != 0x30 {
        return None;
    }
    let (issuer_tag, issuer, _) = tlv(rest)?;
    (issuer_tag == 0x30).then_some(issuer)
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
    let Some(found) = err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    else {
        return;
    };
    let Some((slot, text)) = classify(found) else {
        return;
    };
    let mut last = LAST[slot].lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| t.elapsed() < EVERY) {
        return;
    }
    *last = Some(Instant::now());
    eprintln!("klatch-server: https: client certificate refused: {text}");
}

/// The log slot and the fixed text (no subject, no serial, nothing of the certificate) for a refusal that the revocation
/// check caused; `None` for every other failure.
fn classify(err: &rustls::Error) -> Option<(usize, &'static str)> {
    use rustls::CertificateError as C;
    match err {
        rustls::Error::InvalidCertificate(C::Revoked) => {
            Some((0, "revoked (listed by the client CRL)"))
        }
        rustls::Error::InvalidCertificate(
            C::ExpiredRevocationList | C::ExpiredRevocationListContext { .. },
        ) => Some((
            1,
            "the CRL has expired (its nextUpdate has passed), so every client certificate of its issuer is refused: replace the CRL file and restart the server (reload without a restart is #1492)",
        )),
        rustls::Error::InvalidCertRevocationList(_) => Some((
            2,
            "the CRL is not usable (a bad signature, an unsupported signature algorithm, or an issuer certificate that may not sign CRLs), so every client certificate of its issuer is refused: replace the CRL file and restart the server (reload without a restart is #1492)",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One DER element with a definite length.
    fn el(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        match content.len() {
            n if n < 0x80 => out.push(n as u8),
            n if n < 0x100 => out.extend([0x81, n as u8]),
            n => out.extend([0x82, (n >> 8) as u8, n as u8]),
        }
        out.extend_from_slice(content);
        out
    }

    /// A CRL's outer shape with the issuer `name` (the content of the issuer SEQUENCE), with or without a version.
    fn crl(name: &[u8], version: bool) -> Vec<u8> {
        let mut tbs = Vec::new();
        if version {
            tbs.extend(el(0x02, &[1]));
        }
        tbs.extend(el(0x30, &el(0x06, &[0x2a])));
        tbs.extend(el(0x30, name));
        tbs.extend(el(0x17, b"700101000000Z"));
        let mut list = el(0x30, &tbs);
        list.extend(el(0x30, &el(0x06, &[0x2a])));
        list.extend(el(0x03, &[0, 1]));
        el(0x30, &list)
    }

    #[test]
    fn the_issuer_is_read_with_or_without_a_version_and_names_differ() {
        assert_eq!(issuer_of(&crl(b"abc", true)), Some(&b"abc"[..]));
        assert_eq!(issuer_of(&crl(b"abc", false)), Some(&b"abc"[..]));
        assert_ne!(issuer_of(&crl(b"abc", true)), issuer_of(&crl(b"abd", true)));
        // a long name (two length bytes)
        let long = vec![7u8; 300];
        assert_eq!(issuer_of(&crl(&long, true)), Some(&long[..]));
    }

    #[test]
    fn a_crl_that_is_cut_short_has_no_readable_issuer() {
        let whole = crl(b"abc", true);
        for cut in 0..whole.len() {
            assert_eq!(issuer_of(&whole[..cut]), None, "cut at {cut}");
        }
        let mut trailing = whole.clone();
        trailing.push(0);
        assert_eq!(issuer_of(&trailing), None, "trailing data");
    }

    #[test]
    fn lengths_that_lie_or_are_not_in_der_are_refused() {
        // a length longer than the input, a four-byte length of a gigabyte, an indefinite length, a length of five
        // bytes, a long form for a short length, a leading zero in the length, and a high tag number
        for bad in [
            &[0x30, 0x10, 0x30][..],
            &[0x30, 0x84, 0x40, 0, 0, 0, 0x30, 0],
            &[0x30, 0x80, 0x30, 0x00, 0x00, 0x00],
            &[0x30, 0x85, 0, 0, 0, 0, 1, 0],
            &[0x30, 0x81, 0x05, 1, 2, 3, 4, 5],
            &[0x30, 0x82, 0x00, 0x90, 0],
            &[0x1f, 0x01, 0x00],
            &[],
        ] {
            assert_eq!(tlv(bad), None, "{bad:02x?}");
            assert_eq!(issuer_of(bad), None);
        }
        // the maximum a usize can hold in four bytes is still refused, not wrapped
        assert_eq!(tlv(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]), None);
    }

    #[test]
    fn deep_nesting_costs_nothing_because_nothing_recurses() {
        let deep = [0x30u8, 0x82, 0xff, 0xff].repeat(50_000);
        let started = Instant::now();
        assert_eq!(issuer_of(&deep), None);
        assert!(started.elapsed() < Duration::from_secs(1));
        // well-formed outer elements around a body of nothing but nested openers: read as flat bytes, never descended into
        let body = vec![0x30u8; 65_000];
        let wrapped = el(0x30, &el(0x30, &body));
        let started = Instant::now();
        let _ = issuer_of(&wrapped);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_owner_is_this_user_or_root_and_nobody_else() {
        assert!(owner_ok(501, 501));
        assert!(owner_ok(0, 501));
        assert!(!owner_ok(502, 501));
        assert!(owner_ok(0, 0));
    }

    #[test]
    fn only_the_revocation_errors_are_logged() {
        use rustls::CertificateError as C;
        let slot = |e: rustls::Error| classify(&e).map(|(s, _)| s);
        assert_eq!(slot(rustls::Error::InvalidCertificate(C::Revoked)), Some(0));
        assert_eq!(
            slot(rustls::Error::InvalidCertificate(C::ExpiredRevocationList)),
            Some(1)
        );
        assert_eq!(
            slot(rustls::Error::InvalidCertRevocationList(
                rustls::CertRevocationListError::BadSignature
            )),
            Some(2)
        );
        assert_eq!(
            slot(rustls::Error::InvalidCertificate(C::UnknownIssuer)),
            None
        );
        assert_eq!(slot(rustls::Error::NoCertificatesPresented), None);
    }
}
