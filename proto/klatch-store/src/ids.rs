//! Message ids: 26 characters of Crockford base32, a 48-bit millisecond timestamp followed by 80 bits that
//! are random per process plus a counter. Sortable by time, unique across processes, no dependencies.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 48 random bits, fixed for the life of the process. `RandomState` is seeded by the operating system.
fn process_bits() -> u64 {
    static BITS: OnceLock<u64> = OnceLock::new();
    *BITS.get_or_init(|| RandomState::new().build_hasher().finish() & 0xFFFF_FFFF_FFFF)
}

fn encode(mut value: u128, chars: usize) -> String {
    let mut out = vec![b'0'; chars];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(out).expect("alphabet is ascii")
}

pub fn new_id(now_ms: u64) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let low = ((process_bits() as u128) << 32) | counter;
    format!(
        "{}{}",
        encode((now_ms & 0xFFFF_FFFF_FFFF) as u128, 10),
        encode(low, 16)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    #[test]
    fn ids_are_26_crockford_characters() {
        let id = new_id(1_700_000_000_000);
        assert_eq!(id.len(), 26);
        assert!(id.chars().all(|c| ALPHABET.contains(c)), "bad id {id}");
    }

    #[test]
    fn ids_are_unique_even_within_one_millisecond() {
        let set: HashSet<_> = (0..10_000).map(|_| new_id(5)).collect();
        assert_eq!(set.len(), 10_000);
    }

    #[test]
    fn ids_sort_by_time() {
        assert!(new_id(1_000) < new_id(2_000));
        assert!(new_id(2_000) < new_id(1_000_000_000_000));
    }

    #[test]
    fn ids_increase_within_one_millisecond_in_one_process() {
        let ids: Vec<_> = (0..1_000).map(|_| new_id(7)).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_timestamp_prefix_encodes_the_time() {
        // 48-bit value 0 encodes as ten zeros
        assert!(new_id(0).starts_with("0000000000"));
        assert!(!new_id(1).starts_with("0000000000"));
    }
}
