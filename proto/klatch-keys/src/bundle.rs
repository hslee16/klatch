//! A worker's credential and key bundle (DESIGN section 32 item 13, #121): the token that proves which member a
//! process is, plus the store key and the label keys its clearance reaches, in one file readable only by its owner.
//! A launcher writes it and hands the process the path; the process never reads the keystore.
//!
//! What this is and is not: the token is a bearer credential the gateway issued, expires and can revoke, so a message
//! can honestly be marked verified. The bundle holds the store key because there is no daemon, so a hostile process
//! of the same user that has the file can open the database directly. It separates labels and attribution; it does
//! not isolate a hostile worker (that needs a separate OS user or a daemon).

use crate::{Error, KEY_LEN, Key, KeySource, label_key_name};
use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const TOKEN_PREFIX: &str = "KLATCH-CRED-";

/// A bearer credential: 32 random bytes. The gateway keeps only its hash.
pub struct Token(Zeroizing<[u8; KEY_LEN]>);

impl Token {
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut b = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(&mut *b)?;
        Ok(Self(b))
    }

    /// What the gateway stores and compares.
    pub fn hash(&self) -> [u8; 32] {
        let d = Sha256::digest([b"klatch credential v1".as_slice(), &*self.0].concat());
        d.into()
    }

    pub fn to_text(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("{TOKEN_PREFIX}{}", BASE32_NOPAD.encode(&*self.0)))
    }

    pub fn parse(text: &str) -> Option<Self> {
        let body = text.trim().strip_prefix(TOKEN_PREFIX)?;
        let bytes = BASE32_NOPAD.decode(body.as_bytes()).ok()?;
        let arr: [u8; KEY_LEN] = bytes.as_slice().try_into().ok()?;
        Some(Self(Zeroizing::new(arr)))
    }

    /// Whether this token is the one whose hash is `stored`, compared in constant time.
    pub fn matches(&self, stored: &[u8]) -> bool {
        stored.len() == 32 && bool::from(self.hash().ct_eq(stored))
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// What a launcher hands a worker.
pub struct Bundle {
    pub member: String,
    pub token: Token,
    pub store_key: Key,
    pub label_keys: Vec<(String, Key)>,
}

fn hex(key: &Key) -> Zeroizing<String> {
    let mut s = Zeroizing::new(String::with_capacity(KEY_LEN * 2));
    for b in key.expose() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn unhex(s: &str) -> Option<Key> {
    if s.len() != KEY_LEN * 2 {
        return None;
    }
    let mut out = [0u8; KEY_LEN];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(Key::from_bytes(out))
}

fn bad(msg: impl Into<String>) -> Error {
    Error::Bad(msg.into())
}

impl Bundle {
    /// Write the bundle to `path`, readable only by its owner from the first byte, refusing to replace a file.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let mut text = Zeroizing::new(String::new());
        text.push_str("klatch-bundle 1\n");
        text.push_str(&format!("member {}\n", self.member));
        text.push_str(&format!("token {}\n", self.token.to_text().as_str()));
        text.push_str(&format!("store {}\n", hex(&self.store_key).as_str()));
        for (label, key) in &self.label_keys {
            text.push_str(&format!("label {label} {}\n", hex(key).as_str()));
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| Error::Store(format!("{}: {e}", path.display())))?;
        f.write_all(text.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| Error::Store(format!("{}: {e}", path.display())))
    }

    /// Read a bundle. A file other users can read is refused, and so is anything not exactly the format.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let meta = std::fs::metadata(path)
            .map_err(|e| Error::Store(format!("{}: {e}", path.display())))?;
        if meta.mode() & 0o077 != 0 {
            return Err(bad(format!(
                "{} is readable by other users (mode {:o}); a key bundle must be chmod 600",
                path.display(),
                meta.mode() & 0o777
            )));
        }
        let text = Zeroizing::new(
            std::fs::read_to_string(path)
                .map_err(|e| Error::Store(format!("{}: {e}", path.display())))?,
        );
        let mut lines = text.lines();
        if lines.next() != Some("klatch-bundle 1") {
            return Err(bad(
                "not a klatch key bundle of a version this klatch knows",
            ));
        }
        let (mut member, mut token, mut store) = (None, None, None);
        let mut labels: Vec<(String, Key)> = Vec::new();
        for line in lines {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts.as_slice() {
                ["member", m] if member.is_none() && klatch_valid_member(m) => {
                    member = Some((*m).to_string())
                }
                ["token", t] if token.is_none() => {
                    token = Some(
                        Token::parse(t).ok_or_else(|| bad("the bundle's token is malformed"))?,
                    )
                }
                ["store", k] if store.is_none() => {
                    store =
                        Some(unhex(k).ok_or_else(|| bad("the bundle's store key is malformed"))?)
                }
                ["label", l, k] if !labels.iter().any(|(n, _)| n == l) => {
                    labels.push((
                        (*l).to_string(),
                        unhex(k).ok_or_else(|| bad("a label key in the bundle is malformed"))?,
                    ));
                }
                _ => return Err(bad("the key bundle has a line this klatch does not accept")),
            }
        }
        Ok(Self {
            member: member.ok_or_else(|| bad("the bundle names no member"))?,
            token: token.ok_or_else(|| bad("the bundle has no token"))?,
            store_key: store.ok_or_else(|| bad("the bundle has no store key"))?,
            label_keys: labels,
        })
    }

    /// The bundle as a key source: it answers for the store key and the label keys of the store named
    /// `store_name`, and for nothing else. It cannot be written to.
    pub fn into_source(self, store_name: &str) -> BundleKeys {
        BundleKeys {
            store_name: store_name.to_string(),
            store_key: self.store_key,
            label_keys: self.label_keys,
        }
    }
}

/// A member name is 1 to 40 characters of letters, digits, dot, dash and underscore.
fn klatch_valid_member(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 40
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A read-only key source backed by a worker's bundle.
pub struct BundleKeys {
    store_name: String,
    store_key: Key,
    label_keys: Vec<(String, Key)>,
}

impl KeySource for BundleKeys {
    fn get(&self, name: &str) -> Result<Option<Key>, Error> {
        if name == self.store_name {
            return Ok(Some(Key::from_bytes(*self.store_key.expose())));
        }
        Ok(self
            .label_keys
            .iter()
            .find(|(label, _)| label_key_name(&self.store_name, label) == name)
            .map(|(_, k)| Key::from_bytes(*k.expose())))
    }
    fn put(&self, _name: &str, _key: &Key) -> Result<(), Error> {
        Err(Error::Store("a worker's key bundle is read-only".into()))
    }
    fn delete(&self, _name: &str) -> Result<(), Error> {
        Err(Error::Store("a worker's key bundle is read-only".into()))
    }
    fn describe(&self) -> &'static str {
        "key bundle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("klatch-bundle-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> Bundle {
        Bundle {
            member: "w-1".into(),
            token: Token::generate().unwrap(),
            store_key: Key::from_bytes([1; 32]),
            label_keys: vec![("private".into(), Key::from_bytes([2; 32]))],
        }
    }

    #[test]
    fn a_token_round_trips_hashes_and_matches_only_itself_in_constant_time() {
        let (a, b) = (Token::generate().unwrap(), Token::generate().unwrap());
        let text = a.to_text();
        assert!(text.starts_with("KLATCH-CRED-"));
        let back = Token::parse(&text).unwrap();
        assert_eq!(back.hash(), a.hash());
        assert!(a.matches(&a.hash()) && !a.matches(&b.hash()));
        assert!(!a.matches(&[0u8; 5]), "a short hash must not match");
        assert_ne!(
            a.hash().to_vec(),
            a.to_text().as_bytes().to_vec(),
            "the stored value is a hash, not the token"
        );
        assert_eq!(format!("{a:?}"), "Token(<redacted>)");
    }

    #[test]
    fn something_that_is_not_a_token_is_refused() {
        for bad in [
            "",
            "KLATCH-CRED-",
            "KLATCH-CRED-AAAA",
            "KLATCH-AAAA",
            "hello",
            "KLATCH-CRED-!!!!",
        ] {
            assert!(Token::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_bundle_round_trips_through_a_file_that_only_its_owner_can_read() {
        let d = dir("rt");
        let p = d.join("w-1.bundle");
        let b = sample();
        b.write(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().mode() & 0o777, 0o600);
        let back = Bundle::read(&p).unwrap();
        assert_eq!(back.member, "w-1");
        assert_eq!(back.token.hash(), b.token.hash());
        assert_eq!(back.store_key, b.store_key);
        assert_eq!(back.label_keys.len(), 1);
        assert_eq!(back.label_keys[0].1, Key::from_bytes([2; 32]));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn writing_never_replaces_a_file_and_reading_refuses_a_file_others_can_read() {
        let d = dir("perm");
        let p = d.join("b");
        sample().write(&p).unwrap();
        assert!(sample().write(&p).is_err(), "a bundle replaced a file");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        let err = Bundle::read(&p)
            .err()
            .expect("read a loose bundle")
            .to_string();
        assert!(err.contains("chmod 600"), "{err}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_malformed_bundle_is_refused_whole_and_its_error_never_prints_a_key() {
        let d = dir("bad");
        let good = {
            let p = d.join("good");
            sample().write(&p).unwrap();
            std::fs::read_to_string(&p).unwrap()
        };
        let cases = [
            good.replace("klatch-bundle 1", "klatch-bundle 2"),
            good.replace("member w-1", "member bad name"),
            format!("{good}extra line\n"),
            format!("{good}member w-2\n"),
            good.lines()
                .filter(|l| !l.starts_with("store"))
                .collect::<Vec<_>>()
                .join("\n"),
            good.replace("store 01", "store zz"),
            format!("{good}label private {}\n", "03".repeat(32)),
        ];
        for (i, text) in cases.iter().enumerate() {
            let p = d.join(format!("case{i}"));
            std::fs::write(&p, text).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            let err = Bundle::read(&p)
                .err()
                .unwrap_or_else(|| panic!("case {i} was accepted"))
                .to_string();
            assert!(
                !err.contains("0101") && !err.contains("0202"),
                "case {i}: {err}"
            );
        }
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn as_a_key_source_it_answers_for_its_own_keys_only_and_cannot_be_written() {
        let src = sample().into_source("store-abc");
        assert_eq!(
            src.get("store-abc").unwrap().unwrap(),
            Key::from_bytes([1; 32])
        );
        assert_eq!(
            src.get("store-abc.label.private").unwrap().unwrap(),
            Key::from_bytes([2; 32])
        );
        assert!(src.get("store-abc.label.secret").unwrap().is_none());
        assert!(src.get("store-other").unwrap().is_none());
        assert!(src.put("store-abc", &Key::from_bytes([9; 32])).is_err());
        assert!(src.delete("store-abc").is_err());
    }
}
