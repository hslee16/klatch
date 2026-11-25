//! The owner check at the call site of the CRL file read (#1622): `read_crls_as` takes the user the server runs as, so a
//! file owned by this user can stand for a file owned by someone else (no root needed). `owner_ok` itself is tested in
//! `tls_crl.rs`; here it is the wiring that is pinned: the file's own owner is compared with the injected user.

use crate::tls_crl::read_crls_as;
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
    let err = read_crls_as(&path, &[], 0, stranger).expect_err("another user's file was read");
    let text = err.to_string();
    assert!(
        text.contains("is not owned by the user running the server or by root"),
        "{text}"
    );
    assert!(text.contains(&path.display().to_string()), "{text}");
    // the file's own owner passes the check, and is stopped later, by what the file holds
    let own = read_crls_as(&path, &[], 0, owner).expect_err("not a CRL");
    assert!(!own.to_string().contains("not owned"), "{own}");
    assert!(
        own.to_string().contains("not valid PEM") || own.to_string().contains("holds no CRL"),
        "{own}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
