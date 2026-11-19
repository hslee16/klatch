//! Crypto tests (#624, DESIGN item 70): property tests on the sealed-value and recovery-code code, and pinned
//! known-answer vectors, so a dependency bump or an edit that changes any output is caught.
//!
//! Evidence, not proof, and not a review: a pass means no counterexample was found in the cases tried and that the
//! outputs still match the pinned values. Nobody outside this project has reviewed the crypto code.
//!
//! The vectors come from three places. The algorithm vectors are the published ones (draft-arciszewski-xchacha-03
//! appendix A.1, FIPS 180-2 and RFC 4648 section 10). The klatch format vectors (a sealed value, its text form, a
//! recovery code, a credential token) were computed by `proto/klatch/tests/crypto-vectors.py`, a small standalone
//! implementation that first reproduces the XChaCha20-Poly1305 vector above, so they do not come from the code under
//! test. Run it to regenerate them; a change to any of them is a format change and needs a version bump.

use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD, HEXLOWER};
use hegel::TestCase;
use hegel::generators as gs;
use klatch::keys::{
    Bundle, KEY_LEN, Key, RecoveryError, Token, label_key_name, protect, protect_text,
    recovery_code, restore, reveal, reveal_text,
};
use klatch::store::MessageStore;
use klatch::store::{Seat, SqliteStore};
use sha2::{Digest, Sha256};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

fn hex(s: &str) -> Vec<u8> {
    HEXLOWER.decode(s.as_bytes()).unwrap()
}

fn bytes(n: usize) -> impl hegel::generators::PrintableGenerator<Vec<u8>> {
    gs::vecs(gs::integers::<u8>()).min_size(n).max_size(n)
}

fn key_of(b: &[u8]) -> Key {
    Key::from_bytes(b.try_into().unwrap())
}

fn any_bytes(tc: &TestCase) -> Vec<u8> {
    tc.draw(gs::vecs(gs::integers::<u8>()).max_size(200))
}

// ---- properties ----

#[hegel::test]
fn sealing_then_opening_gives_back_what_was_sealed(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let (context, plain) = (any_bytes(&tc), any_bytes(&tc));
    let blob = protect(&key, &context, &plain).unwrap();
    assert_eq!(reveal(&key, &context, &blob).unwrap(), plain);
}

#[hegel::test]
fn sealed_text_opens_to_the_same_text(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let context = any_bytes(&tc);
    let text = tc.draw(gs::text().max_size(100));
    let sealed = protect_text(&key, &context, &text).unwrap();
    assert_eq!(reveal_text(&key, &context, &sealed).unwrap(), text);
}

#[hegel::test]
fn a_sealed_value_with_any_one_bit_flipped_does_not_open(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let (context, plain) = (any_bytes(&tc), any_bytes(&tc));
    let mut blob = protect(&key, &context, &plain).unwrap();
    let at = tc.draw(gs::integers::<u32>()) as usize % blob.len();
    blob[at] ^= 1 << (tc.draw(gs::integers::<u8>()) % 8);
    assert!(
        reveal(&key, &context, &blob).is_err(),
        "a value with bit changed at byte {at} still opened"
    );
}

#[hegel::test]
fn a_sealed_value_cut_short_or_extended_does_not_open(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let (context, plain) = (any_bytes(&tc), any_bytes(&tc));
    let mut blob = protect(&key, &context, &plain).unwrap();
    if tc.draw(gs::booleans()) {
        blob.truncate(tc.draw(gs::integers::<u32>()) as usize % blob.len());
    } else {
        blob.extend(tc.draw(gs::vecs(gs::integers::<u8>()).min_size(1).max_size(20)));
    }
    assert!(reveal(&key, &context, &blob).is_err());
}

#[hegel::test]
fn a_sealed_value_does_not_open_under_another_key_or_another_context(tc: TestCase) {
    let (a, b) = (tc.draw(bytes(KEY_LEN)), tc.draw(bytes(KEY_LEN)));
    let (context, other) = (any_bytes(&tc), any_bytes(&tc));
    let blob = protect(&key_of(&a), &context, &any_bytes(&tc)).unwrap();
    if a != b {
        assert!(reveal(&key_of(&b), &context, &blob).is_err(), "wrong key");
    }
    if context != other {
        assert!(reveal(&key_of(&a), &other, &blob).is_err(), "wrong context");
    }
}

#[hegel::test]
fn changed_sealed_text_does_not_open_to_anything_else(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let context = any_bytes(&tc);
    let text = tc.draw(gs::text().max_size(60));
    let sealed = protect_text(&key, &context, &text).unwrap();
    let mut chars: Vec<char> = sealed.chars().collect();
    let at = tc.draw(gs::integers::<u32>()) as usize % chars.len();
    let replacement = tc.draw(gs::sampled_from(
        "ABCxyz019-_=:+/ ".chars().collect::<Vec<_>>(),
    ));
    chars[at] = replacement;
    let changed: String = chars.into_iter().collect();
    // the same text back (a spelling that decodes to the same bytes) is fine; any other outcome must be a refusal
    if let Ok(opened) = reveal_text(&key, &context, &changed) {
        assert_eq!(opened, text, "changed text opened to something else");
    }
}

#[hegel::test]
fn a_recovery_code_restores_its_key_however_it_is_typed(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let code = recovery_code(&key);
    assert_eq!(restore(&code).unwrap(), key);
    assert_eq!(restore(&code.to_lowercase()).unwrap(), key);
    assert_eq!(restore(&code.replace('-', " ")).unwrap(), key);
    assert_eq!(restore(&code.replace('-', "")).unwrap(), key);
}

fn spelling(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}

#[hegel::test]
fn a_recovery_code_with_one_character_changed_never_gives_back_the_original_key(tc: TestCase) {
    let key = key_of(&tc.draw(bytes(KEY_LEN)));
    let code = recovery_code(&key);
    let mut chars: Vec<char> = code.chars().collect();
    let at = tc.draw(gs::integers::<u32>()) as usize % chars.len();
    let alphabet: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567-0189".chars().collect();
    chars[at] = tc.draw(gs::sampled_from(alphabet));
    let changed: String = chars.into_iter().collect();
    if spelling(&changed) == spelling(&code) {
        return; // the same code spelled differently
    }
    // The check is 16 bits, so a changed code can occasionally decode to a different key without complaint (the
    // exhaustive test below shows it for the pinned code); it must never decode to the original key.
    if let Ok(other) = restore(&changed) {
        assert_ne!(other, key, "a changed code restored the original key");
    }
}

#[hegel::test]
fn comparing_keys_agrees_with_comparing_their_bytes(tc: TestCase) {
    let a = tc.draw(bytes(KEY_LEN));
    let mut b = a.clone();
    if tc.draw(gs::booleans()) {
        let at = tc.draw(gs::integers::<u32>()) as usize % KEY_LEN;
        b[at] ^= 1 << (tc.draw(gs::integers::<u8>()) % 8);
    } else if tc.draw(gs::booleans()) {
        b = tc.draw(bytes(KEY_LEN));
    }
    assert_eq!(key_of(&a) == key_of(&b), a == b);
}

#[hegel::test]
fn token_matching_agrees_with_comparing_the_stored_hash(tc: TestCase) {
    let raw = tc.draw(bytes(KEY_LEN));
    let token = Token::parse(&format!("KLATCH-CRED-{}", BASE32_NOPAD.encode(&raw))).unwrap();
    let mut stored: Vec<u8> = token.hash().to_vec();
    match tc.draw(gs::integers::<u8>()) % 4 {
        0 => {}
        1 => {
            stored[tc.draw(gs::integers::<u32>()) as usize % 32] ^=
                1 << (tc.draw(gs::integers::<u8>()) % 8)
        }
        2 => stored.truncate(tc.draw(gs::integers::<u32>()) as usize % 33),
        _ => stored = tc.draw(gs::vecs(gs::integers::<u8>()).max_size(40)),
    }
    assert_eq!(
        token.matches(&stored),
        token.hash().as_slice() == stored.as_slice()
    );
}

// ---- known answers: the algorithms ----

/// draft-arciszewski-xchacha-03 appendix A.1 (the vector the RustCrypto crate tests itself with).
#[test]
fn xchacha20_poly1305_gives_the_published_answer() {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};
    let key = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let nonce = hex("404142434445464748494a4b4c4d4e4f5051525354555657");
    let aad = hex("50515253c0c1c2c3c4c5c6c7");
    let plain = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let want = hex(concat!(
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb731c7f1b0b4aa6440bf3a82f4eda7e39",
        "ae64c6708c54c216cb96b72e1213b4522f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9",
        "21f9664c97637da9768812f615c68b13b52e",
        "c0875924c1c7987947deafd8780acf49"
    ));
    let cipher = XChaCha20Poly1305::new(key.as_slice().into());
    let got = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .unwrap();
    assert_eq!(got, want);
    let back = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &want,
                aad: &aad,
            },
        )
        .unwrap();
    assert_eq!(back, plain);
}

/// FIPS 180-2 appendix B and the empty message.
#[test]
fn sha256_gives_the_published_answers() {
    let d = |m: &[u8]| HEXLOWER.encode(&Sha256::digest(m));
    assert_eq!(
        d(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        d(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        d(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

/// RFC 4648 section 10 (padding removed, as klatch writes both encodings).
#[test]
fn base32_and_base64url_give_the_published_answers() {
    let b32 = [
        ("", ""),
        ("f", "MY"),
        ("fo", "MZXQ"),
        ("foo", "MZXW6"),
        ("foob", "MZXW6YQ"),
        ("fooba", "MZXW6YTB"),
        ("foobar", "MZXW6YTBOI"),
    ];
    let b64 = [
        ("", ""),
        ("f", "Zg"),
        ("fo", "Zm8"),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg"),
        ("fooba", "Zm9vYmE"),
        ("foobar", "Zm9vYmFy"),
    ];
    for (plain, want) in b32 {
        assert_eq!(BASE32_NOPAD.encode(plain.as_bytes()), want);
        assert_eq!(
            BASE32_NOPAD.decode(want.as_bytes()).unwrap(),
            plain.as_bytes()
        );
    }
    for (plain, want) in b64 {
        assert_eq!(BASE64URL_NOPAD.encode(plain.as_bytes()), want);
        assert_eq!(
            BASE64URL_NOPAD.decode(want.as_bytes()).unwrap(),
            plain.as_bytes()
        );
    }
    // the url-safe alphabet: 0xfb 0xff encodes with '-' and '_' where the standard one has '+' and '/'
    assert_eq!(BASE64URL_NOPAD.encode(&[0xfb, 0xff]), "-_8");
}

// ---- known answers: klatch's own formats (from tests/crypto-vectors.py) ----

fn counting_key() -> Key {
    key_of(&(0..KEY_LEN as u8).collect::<Vec<_>>())
}

const CONTEXT: &[u8] = b"message:lobby";
const PLAIN: &[u8] = b"the code is hunter2";
const SEALED_BLOB: &str = "4b530101a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b731846f3dd432c0c941065ea91945cdb45ed8ad91f959c569f3d46c1674e46548ef4153";
const SEALED_TEXT: &str = "klatch-sealed:1:S1MBAaChoqOkpaanqKmqq6ytrq-wsbKztLW2tzGEbz3UMsDJQQZeqRlFzbRe2K2R-VnFafPUbBZ05GVI70FT";
const RECOVERY_CODE: &str =
    "KLATCH-AAAQ-EAYE-AUDA-OCAJ-BIFQ-YDIO-B4IB-CEQT-CQKR-MFYY-DENB-WHA5-DYPT-JLI";

#[test]
fn a_pinned_sealed_value_still_opens_and_a_new_one_has_the_same_layout() {
    let blob = hex(SEALED_BLOB);
    assert_eq!(reveal(&counting_key(), CONTEXT, &blob).unwrap(), PLAIN);
    assert!(reveal(&counting_key(), b"message:other", &blob).is_err());
    // layout: 'K' 'S', version 1, algorithm 1, a 24 byte nonce, the ciphertext, a 16 byte tag
    assert_eq!(&blob[..4], &[b'K', b'S', 1, 1]);
    let fresh = protect(&counting_key(), CONTEXT, PLAIN).unwrap();
    assert_eq!(&fresh[..4], &blob[..4]);
    assert_eq!(fresh.len(), blob.len());
    assert_eq!(fresh.len(), 4 + 24 + PLAIN.len() + 16);
    // a nonce reused under one key is a break of the cipher: two seals of one value must not share a nonce
    let again = protect(&counting_key(), CONTEXT, PLAIN).unwrap();
    assert_ne!(
        &fresh[4..28],
        &again[4..28],
        "the nonce must be fresh every time"
    );
}

#[test]
fn a_pinned_sealed_text_still_opens_and_a_new_one_has_the_same_form() {
    assert_eq!(
        reveal_text(&counting_key(), CONTEXT, SEALED_TEXT).unwrap(),
        std::str::from_utf8(PLAIN).unwrap()
    );
    let fresh = protect_text(&counting_key(), CONTEXT, "the code is hunter2").unwrap();
    assert!(fresh.starts_with("klatch-sealed:1:"));
    assert_eq!(fresh.len(), SEALED_TEXT.len());
    assert_eq!(
        BASE64URL_NOPAD
            .decode(&SEALED_TEXT.as_bytes()["klatch-sealed:1:".len()..])
            .unwrap(),
        hex(SEALED_BLOB)
    );
}

#[test]
fn the_recovery_code_of_a_pinned_key_is_the_pinned_code() {
    assert_eq!(recovery_code(&counting_key()), RECOVERY_CODE);
    assert_eq!(restore(RECOVERY_CODE).unwrap(), counting_key());
    assert_eq!(
        restore(&RECOVERY_CODE.to_lowercase()).unwrap(),
        counting_key()
    );
}

#[test]
fn every_single_character_change_to_the_pinned_code_is_refused_or_gives_another_key() {
    let chars: Vec<char> = RECOVERY_CODE.chars().collect();
    let mut silent = 0;
    for at in 0..chars.len() {
        for sub in "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".chars() {
            if sub == chars[at] {
                continue;
            }
            let mut c = chars.clone();
            c[at] = sub;
            let code: String = c.into_iter().collect();
            match restore(&code) {
                Ok(k) => {
                    assert_ne!(k, counting_key(), "{code} restored the original key");
                    silent += 1;
                }
                Err(RecoveryError::Malformed | RecoveryError::Mistyped) => {}
            }
        }
    }
    // the check is 16 bits: about one change in 65536 slips through to a different key. Pin how many do, so a change
    // to the check (its length or its input) shows up here.
    assert_eq!(
        silent, 0,
        "{silent} changed codes decoded to some other key"
    );
}

#[test]
fn a_pinned_credential_hashes_and_prints_as_pinned() {
    let text = "KLATCH-CRED-AAAQEAYEAUDAOCAJBIFQYDIOB4IBCEQTCQKRMFYYDENBWHA5DYPQ";
    let token = Token::parse(text).unwrap();
    assert_eq!(
        HEXLOWER.encode(&token.hash()),
        "1980803f12a92800e0b518de617db18c4f8eb5b0e5ca1fb0897ef266cbcc0ce5"
    );
    assert_eq!(token.to_text().as_str(), text);
    assert!(token.matches(&token.hash()));
}

#[test]
fn the_sqlcipher_key_of_a_pinned_key_is_the_pinned_pragma() {
    assert_eq!(
        counting_key().sqlcipher_pragma().as_str(),
        "x'000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'"
    );
}

// ---- #622: key bundles, label key names and archive files ----

fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "klatch-crypto622-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const BUNDLE_TEXT: &str = "klatch-bundle 1\nmember w-1\ntoken KLATCH-CRED-AAAQEAYEAUDAOCAJBIFQYDIOB4IBCEQTCQKRMFYYDENBWHA5DYPQ\nstore 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\nlabel private 1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100\n";

fn bundle_file(dir: &std::path::Path, name: &str, text: &[u8]) -> PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    f.write_all(text).unwrap();
    path
}

#[test]
fn a_pinned_key_bundle_reads_to_the_pinned_fields_and_writes_back_to_the_same_bytes() {
    let dir = scratch("bundle-pinned");
    let b = Bundle::read(&bundle_file(&dir, "in", BUNDLE_TEXT.as_bytes())).unwrap();
    assert_eq!(b.member, "w-1");
    assert_eq!(
        HEXLOWER.encode(&b.token.hash()),
        "1980803f12a92800e0b518de617db18c4f8eb5b0e5ca1fb0897ef266cbcc0ce5"
    );
    assert!(b.store_key == counting_key());
    assert_eq!(b.label_keys.len(), 1);
    assert_eq!(b.label_keys[0].0, "private");
    let rev: Vec<u8> = (0..KEY_LEN as u8).rev().collect();
    assert!(b.label_keys[0].1 == key_of(&rev));
    let out = dir.join("out");
    b.write(&out).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), BUNDLE_TEXT);
}

#[hegel::test(test_cases = 20)]
fn a_key_bundle_reads_back_what_was_written(tc: TestCase) {
    let dir = scratch("bundle-rt");
    let token = Token::parse(&format!(
        "KLATCH-CRED-{}",
        BASE32_NOPAD.encode(&tc.draw(bytes(KEY_LEN)))
    ))
    .unwrap();
    let store_raw = tc.draw(bytes(KEY_LEN));
    let label_raw = tc.draw(bytes(KEY_LEN));
    let bundle = Bundle {
        member: "m.1_x-2".into(),
        token,
        store_key: key_of(&store_raw),
        label_keys: vec![("secret".into(), key_of(&label_raw))],
    };
    let path = dir.join("b");
    bundle.write(&path).unwrap();
    let back = Bundle::read(&path).unwrap();
    assert_eq!(back.member, bundle.member);
    assert_eq!(back.token.hash(), bundle.token.hash());
    assert!(back.store_key == bundle.store_key);
    assert!(back.label_keys[0].1 == bundle.label_keys[0].1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn any_one_bit_flipped_in_a_key_bundle_is_refused_or_reads_as_a_different_bundle() {
    let dir = scratch("bundle-flip");
    let original = Bundle::read(&bundle_file(&dir, "orig", BUNDLE_TEXT.as_bytes())).unwrap();
    let mut silent = 0;
    for at in 0..BUNDLE_TEXT.len() {
        for bit in 0..8 {
            let mut text = BUNDLE_TEXT.as_bytes().to_vec();
            // a hex number is read in either case, so a case flip of a-f is the same key (not an integrity gap: a
            // bundle is a plain 0600 file with no checksum, and what it holds is the keys themselves)
            if bit == 5 && matches!(text[at], b'a'..=b'f') && at > 100 {
                continue;
            }
            text[at] ^= 1 << bit;
            let path = bundle_file(&dir, &format!("f{at}-{bit}"), &text);
            let same = match Bundle::read(&path) {
                Err(_) => false,
                Ok(b) => {
                    b.member == original.member
                        && b.token.hash() == original.token.hash()
                        && b.store_key == original.store_key
                        && b.label_keys == original.label_keys
                }
            };
            if same {
                silent += 1;
            }
            std::fs::remove_file(path).unwrap();
        }
    }
    assert_eq!(silent, 0, "{silent} flips read back as the original bundle");
}

#[test]
fn label_key_names_are_pinned_and_always_inside_the_key_name_alphabet() {
    assert_eq!(label_key_name("store", "private"), "store.label.private");
    assert_eq!(label_key_name("s", "a b/c.d"), "s.label.a_b_c_d");
    assert_eq!(label_key_name("s", "Ok-1_x"), "s.label.Ok-1_x");
}

#[hegel::test]
fn a_label_key_name_is_deterministic_and_safe_for_any_label(tc: TestCase) {
    let label = tc.draw(gs::text().max_size(40));
    let name = label_key_name("store", &label);
    assert_eq!(name, label_key_name("store", &label));
    let tail = name.strip_prefix("store.label.").unwrap();
    assert_eq!(tail.chars().count(), label.chars().count());
    assert!(
        tail.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    );
}

/// An archive file of two messages, written under `key`.
fn an_archive(dir: &std::path::Path, key: &Key) -> PathBuf {
    let s = SqliteStore::open(&dir.join("live.db"), key).unwrap();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "alice"), "archived-needle-one", 10)
        .unwrap();
    s.post(Seat::new("r", "alice"), "archived-needle-two", 11)
        .unwrap();
    let out = dir.join("a.db");
    assert_eq!(s.archive_before("r", 500, &out, key).unwrap(), 2);
    out
}

#[test]
fn an_archive_file_holds_no_plaintext_and_opens_only_under_its_key() {
    let dir = scratch("archive-key");
    let key = counting_key();
    let file = an_archive(&dir, &key);
    let raw = std::fs::read(&file).unwrap();
    assert!(!raw.starts_with(b"SQLite format 3"));
    for needle in ["archived-needle", "alice"] {
        assert!(!raw.windows(needle.len()).any(|w| w == needle.as_bytes()));
    }
    let other = key_of(&[9u8; KEY_LEN]);
    let live = SqliteStore::open(&dir.join("live.db"), &key).unwrap();
    assert!(live.restore_archive(&file, &other).is_err());
    assert_eq!(live.restore_archive(&file, &key).unwrap(), 2);
}

#[test]
fn a_bit_flipped_anywhere_in_an_archive_file_is_never_restored_as_data() {
    let dir = scratch("archive-flip");
    let key = counting_key();
    let file = an_archive(&dir, &key);
    let good = std::fs::read(&file).unwrap();
    // the first byte, the salt, the middle and last byte of every 4096 byte page, and the very end
    let mut spots: Vec<usize> = vec![0, 15, 16, 100];
    for page in 0..good.len() / 4096 {
        spots.extend([page * 4096 + 2000, page * 4096 + 4095]);
    }
    spots.push(good.len() - 1);
    let live = SqliteStore::open(&dir.join("live.db"), &key).unwrap();
    for at in spots {
        let mut bad = good.clone();
        bad[at] ^= 0x10;
        let path = dir.join(format!("bad{at}.db"));
        std::fs::write(&path, &bad).unwrap();
        // refused outright, or it restores nothing (the live rows are tombstones of exactly these messages)
        let restored = live.restore_archive(&path, &key);
        assert!(
            restored.is_err(),
            "a flip at byte {at} of {} was accepted: {restored:?}",
            good.len()
        );
    }
}
