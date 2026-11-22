//! The admin passphrase verifier (#1233): an Argon2id hash with its salt and its cost parameters, kept as one policy
//! value so the parameters can be raised later without breaking an old value. The passphrase itself is never kept.
//!
//! Value format (ASCII, one line): `argon2id$m=<KiB>,t=<passes>,p=<lanes>$<salt hex>$<hash hex>`.

use argon2::{Algorithm, Argon2, Params, Version};
use data_encoding::HEXLOWER;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// The shortest passphrase `init-passphrase` accepts, in bytes. A floor against an empty or trivial one; the real
/// defence against guessing is the cost parameters and the failure limit.
pub const MIN_LEN: usize = 10;
/// The longest, in bytes: bounds the work and the buffer.
pub const MAX_LEN: usize = 256;

const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;

/// The Argon2id cost: the OWASP baseline (19 MiB, 2 passes, 1 lane). Raise it here; an older stored value keeps
/// verifying under the parameters it records and is rewritten under these at the next successful proof.
pub const CURRENT: Cost = Cost {
    m_kib: 19 * 1024,
    t: 2,
    p: 1,
};

/// The most a stored value may ask of this process, so a damaged or hostile policy row cannot make a proof allocate
/// without bound.
const MAX_M_KIB: u32 = 1024 * 1024;
const MAX_T: u32 = 16;
const MAX_P: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

/// A parsed verifier.
#[derive(Clone, Debug)]
pub struct Verifier {
    cost: Cost,
    salt: [u8; SALT_LEN],
    hash: [u8; HASH_LEN],
}

fn derive(pass: &[u8], salt: &[u8], cost: Cost) -> Option<Zeroizing<[u8; HASH_LEN]>> {
    let params = Params::new(cost.m_kib, cost.t, cost.p, Some(HASH_LEN)).ok()?;
    let mut out = Zeroizing::new([0u8; HASH_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pass, salt, &mut *out)
        .ok()?;
    Some(out)
}

/// Why a passphrase may not be set, or `Ok`.
pub fn check_new(pass: &str) -> Result<(), String> {
    if pass.len() < MIN_LEN {
        return Err(format!(
            "the passphrase must be at least {MIN_LEN} characters"
        ));
    }
    if pass.len() > MAX_LEN {
        return Err(format!("the passphrase must be at most {MAX_LEN} bytes"));
    }
    if pass.trim() != pass {
        return Err("the passphrase must not start or end with a space".into());
    }
    Ok(())
}

impl Verifier {
    /// A new verifier of `pass` under `cost` and a fresh random salt.
    pub fn create(pass: &str, cost: Cost) -> Result<Self, String> {
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|e| format!("no randomness: {e}"))?;
        let hash = derive(pass.as_bytes(), &salt, cost).ok_or("bad passphrase cost")?;
        Ok(Self {
            cost,
            salt,
            hash: *hash,
        })
    }

    /// Whether `pass` is the passphrase. The comparison is constant time; an empty or over-long attempt is false
    /// without hashing.
    pub fn verify(&self, pass: &str) -> bool {
        if pass.is_empty() || pass.len() > MAX_LEN {
            return false;
        }
        match derive(pass.as_bytes(), &self.salt, self.cost) {
            Some(h) => bool::from(h.ct_eq(&self.hash)),
            None => false,
        }
    }

    /// The parameters it was made under.
    pub fn cost(&self) -> Cost {
        self.cost
    }

    /// Whether it was made under weaker parameters than `CURRENT`, so the next success should rewrite it.
    pub fn is_stale(&self) -> bool {
        self.cost.m_kib < CURRENT.m_kib || self.cost.t < CURRENT.t || self.cost.p < CURRENT.p
    }

    /// An identifier of this exact verifier (its salt): an unlock earned under one is void under any other.
    pub fn id(&self) -> String {
        HEXLOWER.encode(&self.salt)
    }

    pub fn encode(&self) -> String {
        format!(
            "argon2id$m={},t={},p={}${}${}",
            self.cost.m_kib,
            self.cost.t,
            self.cost.p,
            HEXLOWER.encode(&self.salt),
            HEXLOWER.encode(&self.hash)
        )
    }

    /// Parse a stored value. Anything that is not exactly the format, or asks for more than the bounds, is `None`
    /// (and a proof then refuses: a damaged verifier never lets anyone in).
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split('$');
        if parts.next()? != "argon2id" {
            return None;
        }
        let cost = parts.next()?;
        let salt = HEXLOWER.decode(parts.next()?.as_bytes()).ok()?;
        let hash = HEXLOWER.decode(parts.next()?.as_bytes()).ok()?;
        if parts.next().is_some() {
            return None;
        }
        let (mut m, mut t, mut p) = (None, None, None);
        for kv in cost.split(',') {
            let (k, v) = kv.split_once('=')?;
            let v: u32 = v.parse().ok()?;
            match k {
                "m" => m = Some(v),
                "t" => t = Some(v),
                "p" => p = Some(v),
                _ => return None,
            }
        }
        let cost = Cost {
            m_kib: m?,
            t: t?,
            p: p?,
        };
        if cost.m_kib < 8
            || cost.m_kib > MAX_M_KIB
            || !(1..=MAX_T).contains(&cost.t)
            || !(1..=MAX_P).contains(&cost.p)
        {
            return None;
        }
        Some(Self {
            cost,
            salt: salt.try_into().ok()?,
            hash: hash.try_into().ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cheap cost so the unit tests are fast; `CURRENT` is exercised once below.
    const CHEAP: Cost = Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };

    #[test]
    fn the_right_passphrase_verifies_and_a_wrong_or_empty_one_does_not() {
        let v = Verifier::create("correct horse battery", CHEAP).unwrap();
        assert!(v.verify("correct horse battery"));
        assert!(!v.verify("correct horse batterz"));
        assert!(!v.verify(""));
        assert!(!v.verify(&"x".repeat(MAX_LEN + 1)));
    }

    #[test]
    fn the_stored_value_never_holds_the_passphrase_and_round_trips() {
        let v = Verifier::create("correct horse battery", CHEAP).unwrap();
        let text = v.encode();
        assert!(!text.contains("correct"), "{text}");
        assert!(text.starts_with("argon2id$m=64,t=1,p=1$"));
        let back = Verifier::parse(&text).unwrap();
        assert!(back.verify("correct horse battery"));
        assert_eq!(back.cost(), CHEAP);
        assert_eq!(back.id(), v.id());
    }

    #[test]
    fn two_verifiers_of_one_passphrase_differ_by_salt() {
        let a = Verifier::create("correct horse battery", CHEAP).unwrap();
        let b = Verifier::create("correct horse battery", CHEAP).unwrap();
        assert_ne!(a.encode(), b.encode());
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn a_damaged_or_out_of_bounds_value_does_not_parse() {
        let good = Verifier::create("correct horse battery", CHEAP)
            .unwrap()
            .encode();
        for bad in [
            "".to_string(),
            "argon2i$m=64,t=1,p=1$00$00".into(),
            good.replace("m=64", "m=4"),
            good.replace("m=64", "m=99999999"),
            good.replace("t=1", "t=0"),
            good.replace("p=1", "p=99"),
            good.replace("t=1,", ""),
            format!("{good}$extra"),
            good.replace("$m=", "$x="),
            good[..good.len() - 2].to_string(),
        ] {
            assert!(Verifier::parse(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_weaker_stored_cost_is_stale_and_the_current_one_is_not() {
        let weak = Verifier::create("correct horse battery", CHEAP).unwrap();
        assert!(weak.is_stale());
        let now = Verifier::create("correct horse battery", CURRENT).unwrap();
        assert!(!now.is_stale());
        assert!(now.verify("correct horse battery"));
    }

    #[test]
    fn a_new_passphrase_must_be_long_enough_and_not_padded() {
        assert!(check_new("correct horse battery").is_ok());
        assert!(check_new("").is_err());
        assert!(check_new("short").is_err());
        assert!(check_new(" spaced out passphrase").is_err());
        assert!(check_new(&"x".repeat(MAX_LEN + 1)).is_err());
    }
}
