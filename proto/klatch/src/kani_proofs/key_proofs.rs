// ---------------------------------------------------------------------------------------------------------------
// The sealed header (keys/sealed.rs) and the recovery code (keys/recovery.rs).
// ---------------------------------------------------------------------------------------------------------------

use crate::keys::{KEY_LEN, Key, RevealError, is_sealed_text, looks_protected, reveal};

/// Header parse. Bound: blobs of 0 to 30 bytes (the header is 28), every byte arbitrary.
/// Not covered: blobs longer than 30 bytes, and whether a well-formed header opens (that is the AEAD, outside
/// the reach of a model checker; the authenticated-encryption property tests cover it).
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(40)]
fn a_blob_is_protected_only_with_the_full_header_and_the_magic() {
    let bytes: [u8; 30] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 30);
    let blob = &bytes[..len];
    assert_eq!(looks_protected(blob), len >= 28 && blob[..2] == *b"KS");
    if len < 28 {
        // short input is refused before any byte past the header is read: no panic, one error
        let key = Key::from_bytes([0; KEY_LEN]);
        assert_eq!(reveal(&key, b"", blob), Err(RevealError::Malformed));
    }
}

/// An unknown version or algorithm is refused by name, before the cipher runs.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(40)]
fn an_unknown_version_or_algorithm_is_refused_before_decryption() {
    let mut bytes: [u8; 30] = kani::any();
    bytes[0] = b'K';
    bytes[1] = b'S';
    let (v, a) = (bytes[2], bytes[3]);
    kani::assume(!(v == 1 && a == 1));
    let key = Key::from_bytes([0; KEY_LEN]);
    let want = if v == 1 {
        RevealError::UnknownAlgorithm(a)
    } else {
        RevealError::UnknownVersion(v)
    };
    assert_eq!(reveal(&key, b"", &bytes), Err(want));
}

/// The text form is recognised by its prefix alone.
/// Bound: twelve concrete texts, no symbolic input: the exact prefix, with and without a body after it, and texts
/// that differ from it in the first byte, a middle byte, the version byte, the last colon, by being one byte short,
/// by being the empty string, and by holding the prefix after other text. Not covered: any other text. Concrete
/// because validating symbolic bytes as UTF-8 did not finish in 600 s (sixteen bytes, one byte at a symbolic
/// position, or forty-eight concrete validations), and `from_utf8_unchecked` is refused by the unsafe-code check
/// in tests/hygiene.rs.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(40)]
fn sealed_text_is_recognised_by_its_prefix() {
    for yes in [
        "klatch-sealed:1:",
        "klatch-sealed:1:AAAA",
        "klatch-sealed:1:klatch-sealed:1:",
    ] {
        assert!(is_sealed_text(yes));
    }
    for no in [
        "",
        "xlatch-sealed:1:",
        "klatch-xealed:1:",
        "klatch-sealed:2:",
        "klatch-sealed:1x",
        "klatch-sealed:1",
        "klatch-sealed:",
        " klatch-sealed:1:",
        "a message that mentions klatch-sealed:1: later",
    ] {
        assert!(!is_sealed_text(no));
    }
}

// The recovery code (keys/recovery.rs), the framing only (#656). `restore` and `recovery_code` also filter and
// upper-case a heap `String`, decode base32 into a `Vec` and hash with SHA-256; a harness over input of only 2
// characters (a real code has 63) ran past 500 s, so that part is not covered. What is covered is `frame_with` and
// `unframe_with`, the functions both call (the code passes the real SHA-256 check; the harnesses pass a stand-in
// check, a function of the key bytes, so what they establish is a property of the framing for any check function and
// says nothing about SHA-256).
use crate::keys::{RecoveryError, frame_with, unframe_with};

/// A stand-in check: xor of the bytes and wrapping sum of the bytes.
fn stand_in(key: &[u8]) -> [u8; 2] {
    let (mut x, mut sum) = (0u8, 0u8);
    for b in key {
        x ^= *b;
        sum = sum.wrapping_add(*b);
    }
    [x, sum]
}

/// Anything but 34 decoded bytes is `Malformed`, before the check is looked at. Bound: 0 to 36 bytes, any content.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(40)]
fn a_recovery_code_of_the_wrong_length_is_malformed() {
    let bytes: [u8; 36] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 36 && len != KEY_LEN + 2);
    assert!(unframe_with(&bytes[..len], stand_in) == Err(RecoveryError::Malformed));
}

/// Of 34 bytes, the code restores exactly when the last two are the check of the first 32, and then to those 32
/// bytes; otherwise it is `Mistyped`. Bound: any 34 bytes.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(40)]
fn the_check_decides_between_a_key_and_a_mistyped_code() {
    let bytes: [u8; 34] = kani::any();
    let want = stand_in(&bytes[..KEY_LEN]);
    let got = unframe_with(&bytes, stand_in);
    if want == [bytes[KEY_LEN], bytes[KEY_LEN + 1]] {
        assert!(got.is_ok() && got.unwrap()[..] == bytes[..KEY_LEN]);
    } else {
        assert!(got == Err(RecoveryError::Mistyped));
    }
}

/// What `recovery_code` frames, `restore` unframes, for every key; and any change to a check byte (a typo in the last
/// four characters, in effect) is caught whatever the check function is, because the check is a function of the key
/// bytes alone. Bound: any 32 byte key, any one changed byte of the two check bytes. Not covered: a change in a key
/// byte, which depends on how well the real check detects it (a unit test tries every character of one code).
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(40)]
fn a_framed_key_round_trips_and_a_changed_check_byte_is_caught() {
    let key: [u8; KEY_LEN] = kani::any();
    let framed = frame_with(&key, stand_in);
    assert!(unframe_with(&framed, stand_in) == Ok(key));
    let at: usize = kani::any();
    kani::assume(at < 2);
    let flip: u8 = kani::any();
    kani::assume(flip != 0);
    let mut changed = framed;
    changed[KEY_LEN + at] ^= flip;
    assert!(unframe_with(&changed, stand_in) == Err(RecoveryError::Mistyped));
}
