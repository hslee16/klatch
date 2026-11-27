//! The owner check at the call site of the CRL file read (#1622): `read_crls_as` takes the user the server runs as, so a
//! file owned by this user can stand for a file owned by someone else (no root needed). `owner_ok` itself is tested in
//! `tls_crl.rs`; here it is the wiring that is pinned: the file's own owner is compared with the injected user.

use crate::tls_crl::{Stage, read_crls_as};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// A file of this user under a directory of its own in the temp directory, mode 0644, holding `text`.
fn file_with(text: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("klatch-crl-owner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("client.crl.pem");
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    (dir, path)
}

#[test]
fn a_file_owned_by_another_user_than_the_one_the_server_runs_as_is_refused_before_anything_is_read()
{
    let (dir, path) = file_with("not a crl, and never read");
    let owner = std::fs::metadata(&path).unwrap().uid();
    if owner == 0 {
        // a test run as root owns every file it makes, and root's files are trusted by design
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let stranger = owner.wrapping_add(1);
    let err = read_crls_as(&path, &[], None, 0, Stage::Start, stranger)
        .expect_err("another user's file was read");
    let text = err.to_string();
    assert!(
        text.contains("is not owned by the user running the server or by root"),
        "{text}"
    );
    assert!(text.contains(&path.display().to_string()), "{text}");
    // the file's own owner passes the check, and is stopped later, by what the file holds
    let own = read_crls_as(&path, &[], None, 0, Stage::Start, owner).expect_err("not a CRL");
    assert!(!own.to_string().contains("not owned"), "{own}");
    assert!(
        own.to_string().contains("not valid PEM") || own.to_string().contains("holds no CRL"),
        "{own}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Standard base64 with padding (the tests need a PEM and have no encoder at hand).
fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// #1704: the lapse rule at a reload, with the clock injected (the integration version slept 6 s on a CRL due in 5 s, and
/// a slow runner could flake it). An expired file is refused while the CRL in force is in date and taken, with an ALERT,
/// once the CRL in force has lapsed: the edge is the second of the in-force `nextUpdate` itself.
#[test]
fn an_expired_file_is_taken_at_a_reload_exactly_when_the_crl_in_force_has_lapsed() {
    use crate::der_tests::{crl_with, el};
    // the CRL in force is due at 2027-01-15 00:00:05 UTC; the file's own nextUpdate is an hour before that
    const IN_FORCE_DUE: i64 = 1_799_971_205;
    let der = crl_with(
        b"ca",
        true,
        el(0x18, b"20270101000000Z"),
        Some(el(0x18, b"20270114230005Z")),
        &[],
    );
    let pem = format!(
        "-----BEGIN X509 CRL-----\n{}\n-----END X509 CRL-----\n",
        base64(&der)
    );
    let (dir, path) = file_with(&pem);
    let owner = std::fs::metadata(&path).unwrap().uid();
    let stage = Stage::Reload {
        in_force_due: Some(IN_FORCE_DUE),
    };
    let ca: &[&[u8]] = &[b"ca"];
    let read = |now: i64| read_crls_as(&path, ca, None, now, stage, owner);
    // one second before the CRL in force lapses: refused, the one in force stays
    let early = read(IN_FORCE_DUE - 1)
        .expect_err("an expired file was taken while the CRL in force is in date");
    assert!(
        early.to_string().contains("while the CRL in force has not"),
        "{early}"
    );
    // at the second it lapses, and after: taken
    for now in [IN_FORCE_DUE, IN_FORCE_DUE + 1] {
        let (crls, earliest) =
            read(now).expect("the file was refused though the CRL in force has lapsed");
        assert_eq!(crls.len(), 1);
        assert_eq!(earliest, Some(IN_FORCE_DUE - 3_600));
    }
    // with no CRL in force at all (none had a nextUpdate) the file is taken too
    let none = Stage::Reload { in_force_due: None };
    assert!(read_crls_as(&path, ca, None, IN_FORCE_DUE - 1, none, owner).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
    // a file due exactly now is expired (the same edge as the notice), so it is refused while the one in force is in date
    let der = crl_with(
        b"ca",
        true,
        el(0x18, b"20270101000000Z"),
        Some(el(0x18, b"20270115000005Z")),
        &[],
    );
    let pem = format!(
        "-----BEGIN X509 CRL-----\n{}\n-----END X509 CRL-----\n",
        base64(&der)
    );
    let (dir, path) = file_with(&pem);
    let in_date = Stage::Reload {
        in_force_due: Some(IN_FORCE_DUE + 10),
    };
    let edge = read_crls_as(&path, ca, None, IN_FORCE_DUE, in_date, owner);
    assert!(edge.is_err(), "a file due this very second was taken");
    let _ = std::fs::remove_dir_all(&dir);
}
