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
        // test modules kept in their own files (`src/.../tests/*.rs`, `tests.rs`) are not shipped code
        .filter(|p| {
            let s = p.to_string_lossy();
            !s.contains("/tests/") && !s.ends_with("/tests.rs")
        })
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

/// The name in a `fn name(` or `fn name<` on this line, if there is one.
fn fn_name(line: &str) -> Option<&str> {
    let rest = line.split("fn ").nth(1)?;
    let end = rest.find(['(', '<'])?;
    Some(rest[..end].trim())
}

#[test]
fn nothing_defines_its_own_cipher_hash_mac_or_key_derivation() {
    let forbidden = [
        "encrypt",
        "decrypt",
        "seal",
        "unseal",
        "cipher",
        "hmac",
        "sha1",
        "sha2",
        "sha256",
        "sha512",
        "md5",
        "aes",
        "chacha",
        "salsa",
        "xor_cipher",
        "rot13",
        "kdf",
        "pbkdf",
        "derive_key",
    ];
    // helpers that only call the audited crates: they ask SQLCipher its version, and seal a message body or payload
    // through `klatch::keys::protect_text`
    let reviewed = ["cipher_version", "seal_body", "seal_text"];
    let mut found = Vec::new();
    for (path, text) in sources() {
        for (n, line) in code_only(&text).lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            let Some(name) = fn_name(t) else { continue };
            let hit = forbidden
                .iter()
                .any(|f| name == *f || name.starts_with(&format!("{f}_")));
            if hit && !reviewed.contains(&name) {
                found.push(format!("{}:{}: {}", path.display(), n + 1, t));
            }
        }
        for (n, line) in code_only(&text).lines().enumerate() {
            let t = line.trim_start();
            if !t.starts_with("//") && (t.contains("const SBOX") || t.contains("static SBOX")) {
                found.push(format!("{}:{}: {}", path.display(), n + 1, t));
            }
        }
    }
    assert!(found.is_empty(), "home-grown crypto?\n{}", found.join("\n"));
}

#[test]
fn the_name_check_catches_what_it_is_meant_to_and_lets_ordinary_names_through() {
    assert_eq!(fn_name("pub fn encrypt(x: u8)"), Some("encrypt"));
    assert_eq!(fn_name("fn seal_block<T>(x: T)"), Some("seal_block"));
    assert_eq!(
        fn_name("    pub fn sealed_labels(&self)"),
        Some("sealed_labels")
    );
    assert_eq!(fn_name("let x = 3;"), None);
}

#[test]
fn unsafe_code_is_only_where_it_was_reviewed() {
    // main.rs: lowering the core dump limit. The harness module: the process table. Nothing else.
    let allowed = ["klatch/src/main.rs", "klatch/src/harness/"];
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
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/keys/key.rs"))
            .unwrap();
    assert!(key.contains("Zeroizing"), "the key is not wiped on drop");
    assert!(
        key.contains("subtle::ConstantTimeEq"),
        "the key is not compared in constant time by an audited crate"
    );
}
