//! Cryptographic hygiene (#162, R36): no home-grown primitives anywhere in the workspace, and `unsafe` only where it
//! has been looked at. A grep, not a proof: it catches the careless case, and review catches the rest.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

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

/// Whether the directory is a crate of the workspace (it has a manifest and is not the build output).
fn is_crate(p: &Path) -> bool {
    p.is_dir() && p.join("Cargo.toml").exists() && p.file_name().is_some_and(|n| n != "target")
}

/// Test modules kept in their own files (`src/.../tests/*.rs`, `tests.rs`) are not shipped code.
fn is_shipped(p: &Path) -> bool {
    let s = p.to_string_lossy();
    !s.contains("/tests/") && !s.ends_with("/tests.rs")
}

/// Every `.rs` file under a crate's `src`, excluding the build output.
fn sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace_root()).unwrap().flatten() {
        if is_crate(&entry.path()) {
            walk(&entry.path().join("src"), &mut files);
        }
    }
    files
        .into_iter()
        .filter(|p| is_shipped(p))
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

/// The non-comment lines of the shipped code of one file, numbered from 1 and trimmed at the start.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    code_only(text)
        .lines()
        .enumerate()
        .map(|(n, line)| (n + 1, line.trim_start()))
        .filter(|(_, t)| !t.starts_with("//"))
}

const FORBIDDEN: [&str; 19] = [
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

/// helpers that only call the audited crates: they ask SQLCipher its version, and seal a message body or payload
/// through `klatch::keys::protect_text`
const REVIEWED: [&str; 3] = ["cipher_version", "seal_body", "seal_text"];

/// Whether a function of this name is named like a primitive and is not one of the reviewed helpers.
fn is_home_grown_name(name: &str) -> bool {
    let hit = FORBIDDEN
        .iter()
        .any(|f| name == *f || name.starts_with(&format!("{f}_")));
    hit && !REVIEWED.contains(&name)
}

fn is_sbox(t: &str) -> bool {
    t.contains("const SBOX") || t.contains("static SBOX")
}

#[test]
fn nothing_defines_its_own_cipher_hash_mac_or_key_derivation() {
    let mut found = Vec::new();
    for (path, text) in sources() {
        let named = code_lines(&text)
            .filter(|(_, t)| fn_name(t).is_some_and(is_home_grown_name))
            .chain(code_lines(&text).filter(|(_, t)| is_sbox(t)));
        found.extend(named.map(|(n, t)| format!("{}:{}: {}", path.display(), n, t)));
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

/// main.rs of both binaries: lowering the core dump limit. The harness module: the process table. The daemon (#102):
/// the peer-credential calls, umask, signal handlers and poll, all libc. The admin passphrase prompt (#1233): termios
/// and read on a terminal descriptor. The tests that drive the binary on a pseudo-terminal (openpty, setsid, poll).
/// The mutual TLS listener (#108), three libc calls with nothing around them: `socketpair` for the in-process pair
/// (`loopback.rs`), `poll` over two descriptors (`tls_bridge.rs`), and `geteuid` for the key file's owner check
/// (`tls.rs`). Nothing else.
const UNSAFE_ALLOWED: [&str; 10] = [
    "klatch-server/src/loopback.rs",
    "klatch-server/src/tls_bridge.rs",
    "klatch-server/src/tls.rs",
    "klatch/src/proof/tty.rs",
    "klatch/tests/common/",
    "klatch-bench/src/pty.rs",
    "klatch/src/main.rs",
    "klatch-server/src/main.rs",
    "klatch-server/src/daemon.rs",
    "klatch/src/harness/",
];

fn is_unsafe(t: &str) -> bool {
    t.contains("unsafe {") || t.contains("unsafe fn") || t.contains("unsafe impl")
}

#[test]
fn unsafe_code_is_only_where_it_was_reviewed() {
    let mut found = Vec::new();
    for (path, text) in sources() {
        let rel = path
            .strip_prefix(workspace_root())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if UNSAFE_ALLOWED.iter().any(|a| rel.contains(a)) {
            continue;
        }
        found.extend(
            code_lines(&text)
                .filter(|(_, t)| is_unsafe(t))
                .map(|(n, _)| format!("{rel}:{n}")),
        );
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
