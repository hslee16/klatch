//! The client certificate revocation list of the mutual TLS listener (#1491, refs #108, #157): `--tls-client-crl FILE`.
//! Names rustls types, so it is one of the TLS modules `tests/no_network.rs` scans.
//!
//! **What the operator supplies.** One PEM file holding one or more X.509 CRLs, signed by the CA (or CAs) of the client CA
//! file. The file is read once at start; SIGHUP reads it again with the same checks (`tls_reload.rs`, #1492), or a restart does.
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
//! small bounded DER reader in `der.rs`: definite lengths only, no recursion, nothing allocated) and refuses a file in which two CRLs
//! share one: replace the file, never append.
//!
//! **What the CRLs must be (#1614).** `thisUpdate` and `nextUpdate` must be valid DER times (`der.rs`: strict UTCTime or
//! GeneralizedTime, not before 1970) and `nextUpdate` must be present (webpki refuses a CRL without one), or the server
//! refuses to start in words that name the file and the CRL's number. These are only WARNINGS, logged at start with a
//! fixed template (the file and the CRL's number, never a subject or serial) while the server starts: a CRL whose
//! `nextUpdate` has passed (it is enforced: its issuer's clients are refused), one due within seven days (`SOON_SECS`),
//! and one whose issuer is the subject of no CA in the client CA file (`ca_subjects`, the trust anchors' subjects, byte
//! for byte as webpki compares): it still applies to clients that present an intermediate CA of that name, which is
//! how an intermediate's CRL works with a root-only CA file, so it is not refused. With `--tls-client-intermediates` (#1623: certificates whose SUBJECT NAMES only match CRL issuers, never trust anchors, never used to build or verify a chain) that warning becomes a refusal for an issuer that is neither a CA subject nor a listed subject. There is no periodic re-check
//! (SIGHUP reloads, #1492).
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
//!   CA that issued none of the clients does nothing, and nothing in rustls can tell; the start-up check above warns of a
//!   CRL whose issuer is in no CA of the file (#1614).
//! - The whole chain is checked, the trust anchor excepted (rustls's default).

use crate::der::{TimeError, issuer_of, next_update_of};
use anyhow::{Result, bail};
use rustls_pki_types::CertificateRevocationListDer;
use rustls_pki_types::pem::PemObject;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The largest CRL file: a CRL of a CA with a very large number of revocations is a few MiB; more is a mistake.
const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The most CRLs one file may hold (one per issuer, and an operator has a handful of CAs).
const MAX_CRLS: usize = 16;

/// When a CRL file is read: at start, or at a reload with the earliest `nextUpdate` of the CRLs in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Nothing older exists: an expired CRL is adopted (and warned of), as the module doc says.
    Start,
    /// A reload. A file whose CRL has expired is REFUSED while the CRLs in force are still in date: CRLs build on each
    /// other, so an expired file is almost always older than the one in force, refusing it loses no revocation, and
    /// adopting it would lock every client of its issuer out. When the CRLs in force have expired too, nothing is lost
    /// by taking the file (and the expiry is an ALERT line).
    Reload { in_force_due: Option<i64> },
}

/// The CRLs of the file at a stage, after its checks (see the module doc), with the earliest `nextUpdate` among them. The
/// error names the file and the failure, never content.
pub fn read_crls_at(
    path: &Path,
    ca_subjects: &[&[u8]],
    intermediates: Option<&[&[u8]]>,
    now: i64,
    stage: Stage,
) -> Result<(Vec<CertificateRevocationListDer<'static>>, Option<i64>)> {
    read_crls_as(
        path,
        ca_subjects,
        intermediates,
        now,
        stage,
        crate::tls::euid(),
    )
}

/// [`read_crls_at`] for a server running as `euid`, which the caller names so that the owner check at the call site can be
/// tested without a file owned by another user (#1622): the only production caller passes the process's own.
pub(crate) fn read_crls_as(
    path: &Path,
    ca_subjects: &[&[u8]],
    intermediates: Option<&[&[u8]]>,
    now: i64,
    stage: Stage,
    euid: u32,
) -> Result<(Vec<CertificateRevocationListDer<'static>>, Option<i64>)> {
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
    if !owner_ok(meta.uid(), euid) {
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
        bail!(
            "the client CRL file {shown} grew beyond 4 MiB while it was being read: it is refused"
        );
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
    let notices = check_cas_and_times(&crls, ca_subjects, intermediates, now, path)?;
    let reload = matches!(stage, Stage::Reload { .. });
    let earliest = crls
        .iter()
        .filter_map(|c| next_update_of(c.as_ref()).ok())
        .min();
    if let Stage::Reload { in_force_due } = stage
        && earliest.is_some_and(|at| at <= now)
        && in_force_due.is_some_and(|due| due > now)
    {
        // an alert that says what happened: the file was not taken, so nothing about its clients changed
        eprintln!(
            "klatch-server: https: client certificate revocation: ALERT: the client CRL file {} has passed its nextUpdate and was not taken: the CRL in force stays",
            path.display()
        );
        bail!(
            "the client CRL file {shown} has passed its nextUpdate while the CRL in force has not: it is refused and the CRL in force stays (an expired list is older than the one in force and would refuse every client certificate of its issuer)"
        );
    }
    for (n, notice) in &notices {
        // an expiry at a reload is an alert: it is a lock-out the operator just caused or kept
        let level = if reload && *notice == Notice::Expired {
            "ALERT"
        } else {
            "warning"
        };
        eprintln!("{}", notice_line_at(level, path, *n, crls.len(), *notice));
    }
    Ok((crls, earliest))
}

/// The time now in seconds since the Unix epoch: read once, by the caller of [`read_crls`], so the checks below are pure.
/// A clock before 1970 is an error (no CRL time can be compared with it), and the server does not start.
pub fn now_unix() -> Result<i64> {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow::anyhow!("the system clock is before 1970: the client CRL times cannot be checked, so the server does not start"))?;
    Ok(i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

/// A CRL due within this many seconds of now draws a warning at start (seven days: time to fetch and restart).
pub const SOON_SECS: i64 = 7 * 86_400;

/// What a CRL earns at start that is a warning only: the server starts (an expired CRL is enforced, which is the
/// documented policy, and the operator is told here instead of at the first refused client).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// `nextUpdate` is now or past: every client certificate of the issuer is refused.
    Expired,
    /// `nextUpdate` is less than [`SOON_SECS`] away.
    ExpiresSoon,
    /// The issuer is the subject of no CA in the client CA file. It still applies to a client that presents an
    /// intermediate CA of that name (webpki matches CRLs to the certificates of the chain), so it is not refused.
    NoCa,
}

/// Refuse a CRL whose issuer or times cannot be read, whose `nextUpdate` is missing (webpki refuses such a CRL, so the
/// server would fail later and less clearly) or whose times are before 1970. Otherwise the warnings: the CRL's 1-based
/// number in the file and its [`Notice`], where `ca_subjects` is the DER content of each CA's subject Name (what
/// rustls-webpki compares with a CRL's issuer, byte for byte). Pure: `now` is given, not read.
fn check_cas_and_times(
    crls: &[CertificateRevocationListDer<'_>],
    ca_subjects: &[&[u8]],
    intermediates: Option<&[&[u8]]>,
    now: i64,
    path: &Path,
) -> Result<Vec<(usize, Notice)>> {
    let shown = path.display();
    let mut notices = Vec::new();
    for (i, crl) in crls.iter().enumerate() {
        let n = i + 1;
        let Some(issuer) = issuer_of(crl.as_ref()) else {
            bail!(
                "the client CRL file {shown} holds a CRL whose issuer cannot be read: it is not a valid X.509 CRL"
            );
        };
        let at = match next_update_of(crl.as_ref()) {
            Ok(at) => at,
            Err(TimeError::MissingNextUpdate) => bail!(
                "the client CRL file {shown} holds a CRL (number {n}) that has no nextUpdate: it is not a valid CRL for this server (the verifier refuses a CRL without one); ask the CA for a CRL with a nextUpdate"
            ),
            Err(TimeError::BeforeEpoch) => bail!(
                "the client CRL file {shown} holds a CRL (number {n}) with a thisUpdate or nextUpdate before 1970, which the verifier does not accept: it is not a valid CRL for this server"
            ),
            Err(TimeError::Unreadable) => bail!(
                "the client CRL file {shown} holds a CRL (number {n}) whose thisUpdate or nextUpdate is not a valid time: it is not a valid X.509 CRL"
            ),
        };
        if !ca_subjects.contains(&issuer) {
            match intermediates {
                // the strict option (#1623): an issuer that is no anchor and no listed intermediate is refused
                Some(listed) if !listed.contains(&issuer) => bail!(
                    "the client CRL file {shown} holds a CRL (number {n}) whose issuer matches no CA in the client CA file and no certificate in the client intermediates file: it would do nothing; list its issuer certificate in the client intermediates file, or remove the CRL"
                ),
                Some(_) => {}
                None => notices.push((n, Notice::NoCa)),
            }
        }
        if at <= now {
            notices.push((n, Notice::Expired));
        } else if at - now <= SOON_SECS {
            notices.push((n, Notice::ExpiresSoon));
        }
    }
    Ok(notices)
}

/// The log line of a warning: a fixed template with the file and the CRL's number, never a subject or a serial.
fn notice_line_at(level: &str, path: &Path, n: usize, of: usize, notice: Notice) -> String {
    let what = match notice {
        Notice::Expired => {
            "has passed its nextUpdate, so every client certificate of its issuer is refused (expiry is enforced): replace the CRL file and send SIGHUP to reload it (or restart the server)"
        }
        Notice::ExpiresSoon => {
            "passes its nextUpdate within 7 days, after which every client certificate of its issuer is refused (expiry is enforced): replace the CRL file and send SIGHUP to reload it (or restart the server) before then"
        }
        Notice::NoCa => {
            "matches no CA in the client CA file; it only applies to clients that present an intermediate CA with that name; if that is not intended it does nothing"
        }
    };
    format!(
        "klatch-server: https: client certificate revocation: {level}: CRL {n} of {of} {what} (client CRL file {})",
        path.display()
    )
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

/// The line printed at start when a CRL is in force.
pub fn start_line(path: &Path) -> String {
    format!(
        "klatch-server: https: client certificate revocation: CRL {} in force (read at start and again on SIGHUP; replace it and send SIGHUP, or restart, to change it; expiry is enforced: when a CRL passes its nextUpdate, every client certificate of its issuer is refused until you replace the CRL file and send SIGHUP (or restart); a certificate no listed CRL covers is allowed)",
        path.display()
    )
}

/// One log line per kind at most this often, so a client that retries a revoked certificate cannot flood the log.
const EVERY: Duration = Duration::from_secs(10);

static LAST: [Mutex<Option<Instant>>; 3] = [const { Mutex::new(None) }; 3];

/// Say, in the server log, why a handshake failed when the cause is the revocation check (and say nothing for any other
/// failure: those are the clients that never had a certificate, and the operator did not ask to hear of each).
pub fn note_refusal(err: &std::io::Error) {
    note_to(err, &LAST, Instant::now(), |text| {
        eprintln!("klatch-server: https: client certificate refused: {text}");
    });
}

/// [`note_refusal`] with the slots, the time and the writer given (the seam the tests use to block the writer).
fn note_to(
    err: &std::io::Error,
    slots: &[Mutex<Option<Instant>>; 3],
    now: Instant,
    say: impl FnOnce(&str),
) {
    let Some(found) = err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    else {
        return;
    };
    let Some((slot, text)) = classify(found) else {
        return;
    };
    if !due(&slots[slot], now) {
        return;
    }
    // written after the lock is released: a blocked stderr must not hold every later refusal at the mutex (#1632)
    say(text);
}

/// Whether a line of this kind may be said at `now` (and, if so, that it was): the lock is held only here.
fn due(slot: &Mutex<Option<Instant>>, now: Instant) -> bool {
    let mut last = slot.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| now.saturating_duration_since(t) < EVERY) {
        return false;
    }
    *last = Some(now);
    true
}

#[cfg(test)]
#[path = "tls_crl_log_tests.rs"]
mod log_tests;

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
            "the CRL has expired (its nextUpdate has passed), so every client certificate of its issuer is refused: replace the CRL file and send SIGHUP to reload it (or restart the server)",
        )),
        rustls::Error::InvalidCertRevocationList(_) => Some((
            2,
            "the CRL is not usable (a bad signature, an unsupported signature algorithm, or an issuer certificate that may not sign CRLs), so every client certificate of its issuer is refused: replace the CRL file and send SIGHUP to reload it (or restart the server)",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::der_tests::{crl_with, el};

    const NOW: i64 = 1_000_000;

    /// A CRL of issuer `name` whose nextUpdate is `due` seconds after `NOW` (`None`: no nextUpdate).
    fn due(name: &[u8], due: Option<i64>) -> CertificateRevocationListDer<'static> {
        // a UTCTime of the whole epoch is not needed: GeneralizedTime from a day count past 1970 is built by hand
        let time = |secs: i64| {
            let days = secs / 86_400;
            let rest = secs % 86_400;
            // 1970 plus `days` days, as a date, via the civil-from-days algorithm
            let z = days + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            el(
                0x18,
                format!(
                    "{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
                    rest / 3600,
                    rest / 60 % 60,
                    rest % 60
                )
                .as_bytes(),
            )
        };
        let next = due.map(|d| time(NOW + d));
        CertificateRevocationListDer::from(crl_with(name, true, time(0), next, &[]))
    }

    fn check(
        crls: &[CertificateRevocationListDer<'_>],
        subjects: &[&[u8]],
    ) -> Result<Vec<(usize, Notice)>> {
        check_cas_and_times(crls, subjects, None, NOW, Path::new("/x/crl.pem"))
    }

    #[test]
    fn a_crl_is_judged_on_its_next_update_at_the_edges() {
        let ca: &[&[u8]] = &[b"ca"];
        let one = |d: i64| check(&[due(b"ca", Some(d))], ca).unwrap();
        // due now or before: expired; one second ahead: soon; exactly seven days: soon; one second more: nothing
        assert_eq!(one(-1), vec![(1, Notice::Expired)]);
        assert_eq!(one(0), vec![(1, Notice::Expired)]);
        assert_eq!(one(1), vec![(1, Notice::ExpiresSoon)]);
        assert_eq!(one(SOON_SECS), vec![(1, Notice::ExpiresSoon)]);
        assert_eq!(one(SOON_SECS + 1), vec![]);
        assert_eq!(one(365 * 86_400), vec![]);
        assert_eq!(SOON_SECS, 7 * 86_400);
    }

    #[test]
    fn a_crl_without_a_next_update_is_refused_and_so_is_one_before_1970() {
        let err = check(&[due(b"ca", None)], &[b"ca"])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("CRL (number 1) that has no nextUpdate"),
            "{err}"
        );
        assert!(err.contains("not a valid CRL for this server"), "{err}");
        // a time before 1970 (a UTCTime year of 69) in either field
        let early = el(0x17, b"690101000000Z");
        let ok = el(0x17, b"700101000000Z");
        for (this, next) in [(early.clone(), Some(ok.clone())), (ok, Some(early))] {
            let crl = CertificateRevocationListDer::from(crl_with(b"ca", true, this, next, &[]));
            let err = check(&[crl], &[b"ca"]).unwrap_err().to_string();
            assert!(err.contains("before 1970"), "{err}");
        }
    }

    #[test]
    fn a_crl_is_matched_to_a_ca_by_its_whole_subject_name_and_a_stranger_only_warns() {
        let crls = [due(b"ca-b", Some(30 * 86_400))];
        assert!(check(&crls, &[b"ca-a", b"ca-b"]).unwrap().is_empty());
        for subjects in [&[][..], &[&b"ca-"[..]], &[b"ca-b2"], &[b"CA-B"], &[b"ca-a"]] {
            assert_eq!(check(&crls, subjects).unwrap(), vec![(1, Notice::NoCa)]);
        }
        // a stranger that is also expired earns both lines
        let both = [due(b"x", Some(-1))];
        assert_eq!(
            check(&both, &[b"ca"]).unwrap(),
            vec![(1, Notice::NoCa), (1, Notice::Expired)]
        );
    }

    #[test]
    fn with_listed_intermediates_a_crl_of_an_unknown_issuer_is_refused_and_a_listed_or_anchor_issuer_is_not()
     {
        let path = Path::new("/x/crl.pem");
        let crls = [due(b"inter", Some(30 * 86_400))];
        let strict = |cas: &[&[u8]], listed: &[&[u8]]| {
            check_cas_and_times(&crls, cas, Some(listed), NOW, path)
        };
        // listed: no warning and no refusal; an anchor of the CA file: the same, whatever is listed
        assert!(strict(&[b"ca"], &[b"inter"]).unwrap().is_empty());
        assert!(strict(&[b"inter"], &[b"other"]).unwrap().is_empty());
        // neither: refused, in words, with the CRL's number and no name
        for (cas, listed) in [
            (&[&b"ca"[..]][..], &[&b"other"[..]][..]),
            (&[], &[]),
            (&[b"ca"], &[b"inte"]),
        ] {
            let err = strict(cas, listed).expect_err("refused").to_string();
            assert!(err.contains("(number 1) whose issuer matches no CA in the client CA file and no certificate in the client intermediates file"), "{err}");
            assert!(
                !err.contains("inter\u{0}") && !err.contains("other"),
                "{err}"
            );
        }
        // the whole name is compared, not a prefix
        assert!(strict(&[b"ca"], &[b"inter-and-more"]).is_err());
        // without the option the same CRL only warns (the behaviour of #1625 is unchanged)
        let warned = check_cas_and_times(&crls, &[b"ca"], None, NOW, path).unwrap();
        assert_eq!(warned, vec![(1, Notice::NoCa)]);
    }

    #[test]
    fn the_warning_names_the_crl_by_number_and_never_by_subject() {
        let crls = [due(b"first", Some(30 * 86_400)), due(b"second", Some(-5))];
        let got = check(&crls, &[b"first", b"second"]).unwrap();
        assert_eq!(got, vec![(2, Notice::Expired)]);
        let path = Path::new("/x/crl.pem");
        for notice in [Notice::Expired, Notice::ExpiresSoon, Notice::NoCa] {
            let line = notice_line_at("warning", path, 2, 3, notice);
            assert!(line.contains("warning: CRL 2 of 3 "), "{line}");
            assert!(line.ends_with("(client CRL file /x/crl.pem)"), "{line}");
        }
        let no_ca = notice_line_at("warning", path, 1, 1, Notice::NoCa);
        assert!(
            no_ca.contains("matches no CA in the client CA file; it only applies to clients that present an intermediate CA with that name; if that is not intended it does nothing"),
            "{no_ca}"
        );
        assert!(!no_ca.contains("list the"), "{no_ca}");
        assert!(
            notice_line_at("warning", path, 1, 1, Notice::Expired)
                .contains("has passed its nextUpdate")
        );
        assert!(
            notice_line_at("warning", path, 1, 1, Notice::ExpiresSoon).contains("within 7 days")
        );
    }

    #[test]
    fn a_crl_with_a_malformed_time_or_no_readable_issuer_is_refused() {
        let ca: &[&[u8]] = &[b"ca"];
        let bad = |this: Vec<u8>, next: Option<Vec<u8>>| {
            CertificateRevocationListDer::from(crl_with(b"ca", true, this, next, &[]))
        };
        let good = el(0x17, b"700101000000Z");
        for crl in [
            bad(good.clone(), Some(el(0x17, b"701301000000Z"))),
            bad(good.clone(), Some(el(0x17, b"700132000000Z"))),
            bad(good.clone(), Some(el(0x18, b"19700101000000.5Z"))),
            bad(good.clone(), Some(el(0x17, b"7001010000"))),
            bad(el(0x17, b"70010100000A"), None),
        ] {
            let err = check(&[crl], ca).unwrap_err().to_string();
            assert!(err.contains("not a valid time"), "{err}");
        }
        let err = check(&[CertificateRevocationListDer::from(vec![0x30, 0x00])], ca)
            .unwrap_err()
            .to_string();
        assert!(err.contains("issuer cannot be read"), "{err}");
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
