//! The recovery code (the human's choice, DESIGN section 32 item 4): the key written once for a person to keep, in
//! groups of four so it can be copied, with a two-byte check so a typo is caught instead of making a wrong key.

use crate::{KEY_LEN, Key};
use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha256};

const PREFIX: &str = "KLATCH";

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryError {
    /// Not a recovery code: wrong prefix or characters, or the wrong length.
    Malformed,
    /// The check did not match: a character was mistyped.
    Mistyped,
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => f.write_str("that is not a klatch recovery code"),
            Self::Mistyped => f.write_str("the recovery code has a mistyped character"),
        }
    }
}
impl std::error::Error for RecoveryError {}

fn check(key: &[u8]) -> [u8; 2] {
    let d = Sha256::digest([b"klatch recovery v1".as_slice(), key].concat());
    [d[0], d[1]]
}

/// The recovery code of `key`: `KLATCH-XXXX-XXXX-...`.
pub fn recovery_code(key: &Key) -> String {
    let mut bytes = key.expose().to_vec();
    bytes.extend(check(key.expose()));
    let text = BASE32_NOPAD.encode(&bytes);
    let groups: Vec<&str> = text
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();
    format!("{PREFIX}-{}", groups.join("-"))
}

/// The key a recovery code stands for. Case, spaces and dashes are ignored.
pub fn restore(code: &str) -> Result<Key, RecoveryError> {
    let cleaned: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let body = cleaned
        .strip_prefix(PREFIX)
        .ok_or(RecoveryError::Malformed)?;
    let bytes = BASE32_NOPAD
        .decode(body.as_bytes())
        .map_err(|_| RecoveryError::Malformed)?;
    if bytes.len() != KEY_LEN + 2 {
        return Err(RecoveryError::Malformed);
    }
    let (key, tail) = bytes.split_at(KEY_LEN);
    if check(key) != [tail[0], tail[1]] {
        return Err(RecoveryError::Mistyped);
    }
    let mut k = [0u8; KEY_LEN];
    k.copy_from_slice(key);
    Ok(Key::from_bytes(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_restores_the_same_key() {
        let key = Key::generate().unwrap();
        let code = recovery_code(&key);
        assert_eq!(restore(&code).unwrap(), key);
    }

    #[test]
    fn a_code_is_grouped_and_prefixed_and_carries_no_lowercase() {
        let code = recovery_code(&Key::from_bytes([1; KEY_LEN]));
        assert!(code.starts_with("KLATCH-"), "{code}");
        assert!(
            code.split('-')
                .skip(1)
                .all(|g| g.len() <= 4 && !g.is_empty())
        );
        assert_eq!(code, code.to_ascii_uppercase());
    }

    #[test]
    fn case_spaces_and_dashes_do_not_matter_when_typing_it_back() {
        let key = Key::generate().unwrap();
        let code = recovery_code(&key);
        let typed = code.to_ascii_lowercase().replace('-', "  ");
        assert_eq!(restore(&typed).unwrap(), key);
        assert_eq!(restore(&code.replace('-', "")).unwrap(), key);
    }

    #[test]
    fn any_single_mistyped_character_is_caught() {
        let code = recovery_code(&Key::from_bytes([9; KEY_LEN]));
        for i in code
            .char_indices()
            .filter(|(_, c)| c.is_ascii_alphanumeric())
            .map(|(i, _)| i)
            .skip(6)
        {
            let mut bytes = code.clone().into_bytes();
            bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
            let bad = String::from_utf8(bytes).unwrap();
            assert!(restore(&bad).is_err(), "typo at {i} was accepted: {bad}");
        }
    }

    #[test]
    fn something_that_is_not_a_code_is_refused() {
        for bad in [
            "",
            "KLATCH",
            "hello",
            "KLATCH-AAAA",
            "OTHER-AAAA-AAAA",
            "KLATCH-1111-1111",
        ] {
            assert!(restore(bad).is_err(), "{bad:?}");
        }
    }
}
