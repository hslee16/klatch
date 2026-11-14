//! Cryptographic hygiene (#162, R36): no home-grown primitives anywhere in the workspace, and `unsafe` only where it
//! has been looked at. A grep, not a proof: it catches the careless case, and review catches the rest.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every `.rs` file under a crate's `src`, excluding the build output.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace_root()).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir()
            && p.join("Cargo.toml").exists()
            && p.file_name().is_some_and(|n| n != "target")
        {
            walk(&p.join("src"), &mut files);
        }
    }
    files
        .into_iter()
        .map(|p| {
            let t = std::fs::read_to_string(&p).unwrap_or_default();
            (p, t)
        })
        .collect()
}

/// Source with the test modules cut off, so a test may name what the code may not.
fn code_only(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or(text)
}

#[test]
fn nothing_defines_its_own_cipher_hash_mac_or_key_derivation() {
    let forbidden = [
        "fn encrypt",
        "fn decrypt",
        "fn seal",
        "fn unseal",
        "fn cipher",
        "fn hmac",
        "fn sha1",
        "fn sha2",
        "fn sha256",
        "fn sha512",
        "fn md5",
        "fn aes",
        "fn chacha",
        "fn salsa",
        "fn xor_cipher",
        "fn rot13",
        "fn kdf",
        "fn pbkdf",
        "fn derive_key",
        "const SBOX",
        "static SBOX",
    ];
    let mut found = Vec::new();
    for (path, text) in sources() {
        for (n, line) in code_only(&text).lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            // `fn cipher_version` only asks SQLCipher which version it is
            if !t.contains("fn cipher_version") && forbidden.iter().any(|f| t.contains(f)) {
                found.push(format!("{}:{}: {}", path.display(), n + 1, t));
            }
        }
    }
    assert!(found.is_empty(), "home-grown crypto?\n{}", found.join("\n"));
}

#[test]
fn unsafe_code_is_only_where_it_was_reviewed() {
    // klatch-cli main.rs: lowering the core dump limit. klatch-harness: the process table. Nothing else.
    let allowed = ["klatch-cli/src/main.rs", "klatch-harness/src/"];
    let mut found = Vec::new();
    for (path, text) in sources() {
        let rel = path
            .strip_prefix(workspace_root())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if allowed.iter().any(|a| rel.contains(a)) {
            continue;
        }
        for (n, line) in code_only(&text).lines().enumerate() {
            let t = line.trim_start();
            if !t.starts_with("//")
                && (t.contains("unsafe {") || t.contains("unsafe fn") || t.contains("unsafe impl"))
            {
                found.push(format!("{rel}:{}", n + 1));
            }
        }
    }
    assert!(found.is_empty(), "unreviewed unsafe code: {found:?}");
}

#[test]
fn the_key_type_wipes_and_compares_through_audited_crates() {
    let key =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/key.rs")).unwrap();
    assert!(key.contains("Zeroizing"), "the key is not wiped on drop");
    assert!(
        key.contains("subtle::ConstantTimeEq"),
        "the key is not compared in constant time by an audited crate"
    );
}
