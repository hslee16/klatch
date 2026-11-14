use zeroize::Zeroizing;

/// The length of every key, in bytes.
pub const KEY_LEN: usize = 32;

/// A secret key. It cannot be printed or copied by accident, compares in constant time and is wiped when dropped.
pub struct Key(Zeroizing<[u8; KEY_LEN]>);

impl Key {
    /// A fresh random key from the operating system.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(&mut *bytes)?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The key as bytes. Handle with care: do not log it, print it or put it in a command line.
    pub fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The key as SQLCipher's raw-key pragma value, `x'<64 hex digits>'`.
    pub fn sqlcipher_pragma(&self) -> Zeroizing<String> {
        let mut s = Zeroizing::new(String::with_capacity(3 + KEY_LEN * 2));
        s.push_str("x'");
        for b in self.0.iter() {
            s.push_str(&format!("{b:02x}"));
        }
        s.push('\'');
        s
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        // constant time: no early exit on the first differing byte
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}
impl Eq for Key {}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_differ_and_have_the_full_length() {
        let (a, b) = (Key::generate().unwrap(), Key::generate().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.expose().len(), KEY_LEN);
        assert!(a.expose().iter().any(|b| *b != 0), "an all-zero key");
    }

    #[test]
    fn a_key_never_prints_its_bytes() {
        let k = Key::from_bytes([0xAB; KEY_LEN]);
        let shown = format!("{k:?} {k:#?}");
        assert!(!shown.contains("ab") && !shown.contains("171"), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn equality_compares_every_byte() {
        let mut other = [7u8; KEY_LEN];
        let k = Key::from_bytes([7u8; KEY_LEN]);
        assert_eq!(k, Key::from_bytes(other));
        other[KEY_LEN - 1] ^= 1;
        assert_ne!(k, Key::from_bytes(other));
        other[KEY_LEN - 1] ^= 1;
        other[0] ^= 1;
        assert_ne!(k, Key::from_bytes(other));
    }

    #[test]
    fn the_sqlcipher_pragma_is_a_quoted_raw_hex_key() {
        let k = Key::from_bytes([0x0F; KEY_LEN]);
        let p = k.sqlcipher_pragma();
        assert_eq!(p.len(), 3 + 64);
        assert!(p.starts_with("x'0f0f") && p.ends_with("0f'"), "{}", p.len());
    }
}
