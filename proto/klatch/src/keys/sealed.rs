//! Authenticated encryption of one value under one key (the field-level layer of DESIGN section 32 item 1).
//!
//! The format carries its own version and algorithm id so an algorithm can change and old values stay readable:
//! `K S <version> <algorithm> <24-byte nonce> <ciphertext and tag>`. Version 1 uses algorithm 1, XChaCha20-Poly1305
//! (RustCrypto), whose 192-bit random nonce is safe to draw at random for every value. The `context` is authenticated
//! but not stored: a value taken from one place and put in another fails to open.

use crate::keys::Key;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

const MAGIC: [u8; 2] = *b"KS";
const VERSION: u8 = 1;
const ALG_XCHACHA20_POLY1305: u8 = 1;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 2 + 1 + 1 + NONCE_LEN;

#[derive(Debug, PartialEq, Eq)]
pub enum RevealError {
    /// Too short to be a value, or not one at all.
    Malformed,
    /// A version this build does not know.
    UnknownVersion(u8),
    /// An algorithm this build does not know.
    UnknownAlgorithm(u8),
    /// Wrong key, wrong context, or the bytes were changed. These are not told apart on purpose.
    Failed,
}

impl std::fmt::Display for RevealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => f.write_str("not a protected value"),
            Self::UnknownVersion(v) => write!(
                f,
                "a protected value of version {v}, which this klatch does not know"
            ),
            Self::UnknownAlgorithm(a) => write!(
                f,
                "a protected value using algorithm {a}, which this klatch does not know"
            ),
            Self::Failed => {
                f.write_str("the value did not open: wrong key, wrong place, or it was changed")
            }
        }
    }
}
impl std::error::Error for RevealError {}

/// Protect `plaintext` under `key`, bound to `context`.
pub fn protect(key: &Key, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, getrandom::Error> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce)?;
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    let body = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: context,
            },
        )
        .map_err(|_| getrandom::Error::UNEXPECTED)?;
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend(MAGIC);
    out.push(VERSION);
    out.push(ALG_XCHACHA20_POLY1305);
    out.extend(nonce);
    out.extend(body);
    Ok(out)
}

const TEXT_PREFIX: &str = "klatch-sealed:1:";

/// `protect` for text that must be stored as text: the value as `klatch-sealed:1:<base64url>`.
pub fn protect_text(
    key: &Key,
    context: &[u8],
    plaintext: &str,
) -> Result<String, getrandom::Error> {
    let blob = protect(key, context, plaintext.as_bytes())?;
    Ok(format!(
        "{TEXT_PREFIX}{}",
        data_encoding::BASE64URL_NOPAD.encode(&blob)
    ))
}

/// Whether text is a value made by `protect_text`.
pub fn is_sealed_text(text: &str) -> bool {
    text.starts_with(TEXT_PREFIX)
}

/// Open text made by `protect_text`.
pub fn reveal_text(key: &Key, context: &[u8], text: &str) -> Result<String, RevealError> {
    let body = text
        .strip_prefix(TEXT_PREFIX)
        .ok_or(RevealError::Malformed)?;
    let blob = data_encoding::BASE64URL_NOPAD
        .decode(body.as_bytes())
        .map_err(|_| RevealError::Malformed)?;
    let plain = reveal(key, context, &blob)?;
    String::from_utf8(plain).map_err(|_| RevealError::Malformed)
}

/// Whether the bytes look like a protected value (a header, not a proof that it opens).
pub fn looks_protected(blob: &[u8]) -> bool {
    blob.len() >= HEADER_LEN && blob[..2] == MAGIC
}

/// Open a value made by `protect`, with the same key and context.
pub fn reveal(key: &Key, context: &[u8], blob: &[u8]) -> Result<Vec<u8>, RevealError> {
    if !looks_protected(blob) {
        return Err(RevealError::Malformed);
    }
    match (blob[2], blob[3]) {
        (VERSION, ALG_XCHACHA20_POLY1305) => {}
        (VERSION, a) => return Err(RevealError::UnknownAlgorithm(a)),
        (v, _) => return Err(RevealError::UnknownVersion(v)),
    }
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    cipher
        .decrypt(
            XNonce::from_slice(&blob[4..HEADER_LEN]),
            Payload {
                msg: &blob[HEADER_LEN..],
                aad: context,
            },
        )
        .map_err(|_| RevealError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(b: u8) -> Key {
        Key::from_bytes([b; 32])
    }

    #[test]
    fn a_value_opens_with_its_key_and_context() {
        let blob = protect(&k(1), b"msg|private", b"the launch code").unwrap();
        assert_eq!(
            reveal(&k(1), b"msg|private", &blob).unwrap(),
            b"the launch code"
        );
    }

    #[test]
    fn the_blob_carries_no_plaintext_and_a_version_and_algorithm_id() {
        let blob = protect(&k(1), b"c", b"visible-text-marker").unwrap();
        assert!(!blob.windows(19).any(|w| w == b"visible-text-marker"));
        assert_eq!(&blob[..4], &[b'K', b'S', 1, 1]);
        assert!(looks_protected(&blob));
    }

    #[test]
    fn the_same_value_protected_twice_differs() {
        let (a, b) = (
            protect(&k(1), b"c", b"x").unwrap(),
            protect(&k(1), b"c", b"x").unwrap(),
        );
        assert_ne!(a, b, "a repeated nonce");
    }

    #[test]
    fn a_wrong_key_a_wrong_context_and_a_changed_byte_all_fail_the_same_way() {
        let blob = protect(&k(1), b"place-a", b"secret").unwrap();
        assert_eq!(reveal(&k(2), b"place-a", &blob), Err(RevealError::Failed));
        assert_eq!(reveal(&k(1), b"place-b", &blob), Err(RevealError::Failed));
        for i in 4..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert_eq!(
                reveal(&k(1), b"place-a", &bad),
                Err(RevealError::Failed),
                "byte {i}"
            );
        }
    }

    #[test]
    fn something_that_is_not_a_value_or_is_of_another_version_is_refused_not_guessed() {
        let blob = protect(&k(1), b"c", b"x").unwrap();
        assert_eq!(reveal(&k(1), b"c", b"short"), Err(RevealError::Malformed));
        assert_eq!(
            reveal(&k(1), b"c", &blob[..10]),
            Err(RevealError::Malformed)
        );
        let mut v2 = blob.clone();
        v2[2] = 2;
        assert_eq!(
            reveal(&k(1), b"c", &v2),
            Err(RevealError::UnknownVersion(2))
        );
        let mut a9 = blob.clone();
        a9[3] = 9;
        assert_eq!(
            reveal(&k(1), b"c", &a9),
            Err(RevealError::UnknownAlgorithm(9))
        );
        assert!(!looks_protected(
            b"plain text that is long enough to be a header and more"
        ));
    }

    #[test]
    fn text_round_trips_as_text_and_announces_itself() {
        let t = protect_text(&k(1), b"c", "héllo wörld").unwrap();
        assert!(is_sealed_text(&t) && t.starts_with("klatch-sealed:1:") && t.is_ascii());
        assert_eq!(reveal_text(&k(1), b"c", &t).unwrap(), "héllo wörld");
        assert_eq!(reveal_text(&k(2), b"c", &t), Err(RevealError::Failed));
        assert_eq!(
            reveal_text(&k(1), b"c", "klatch-sealed:1:!!!"),
            Err(RevealError::Malformed)
        );
        assert_eq!(
            reveal_text(&k(1), b"c", "plain"),
            Err(RevealError::Malformed)
        );
        assert!(!is_sealed_text(
            "a message that mentions klatch-sealed:1: later"
        ));
    }

    #[test]
    fn an_empty_value_and_a_large_one_round_trip() {
        for len in [0usize, 1, 1000, 100_000] {
            let data = vec![0xA5u8; len];
            let blob = protect(&k(3), b"c", &data).unwrap();
            assert_eq!(reveal(&k(3), b"c", &blob).unwrap(), data);
        }
    }
}
