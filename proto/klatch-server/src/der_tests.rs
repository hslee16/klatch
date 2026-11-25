//! Tests of the bounded DER reader (`der.rs`): the issuer, the validity times, and malformed input of every kind.

use crate::der::*;
use std::time::{Duration, Instant};

/// One DER element with a definite length.
pub fn el(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend([0x81, n as u8]),
        n => out.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(content);
    out
}

/// A CRL's outer shape with the issuer `name` (the content of the issuer SEQUENCE), with or without a version.
fn crl(name: &[u8], version: bool) -> Vec<u8> {
    crl_with(name, version, el(0x17, b"700101000000Z"), None, &[])
}

/// A CRL with the given `thisUpdate` element, optional `nextUpdate` element and bytes after them (revokedCertificates,
/// extensions, or junk).
pub fn crl_with(
    name: &[u8],
    version: bool,
    this: Vec<u8>,
    next: Option<Vec<u8>>,
    tail: &[u8],
) -> Vec<u8> {
    let mut tbs = Vec::new();
    if version {
        tbs.extend(el(0x02, &[1]));
    }
    tbs.extend(el(0x30, &el(0x06, &[0x2a])));
    tbs.extend(el(0x30, name));
    tbs.extend(this);
    tbs.extend(next.unwrap_or_default());
    tbs.extend_from_slice(tail);
    let mut list = el(0x30, &tbs);
    list.extend(el(0x30, &el(0x06, &[0x2a])));
    list.extend(el(0x03, &[0, 1]));
    el(0x30, &list)
}

#[test]
fn the_issuer_is_read_with_or_without_a_version_and_names_differ() {
    assert_eq!(issuer_of(&crl(b"abc", true)), Some(&b"abc"[..]));
    assert_eq!(issuer_of(&crl(b"abc", false)), Some(&b"abc"[..]));
    assert_ne!(issuer_of(&crl(b"abc", true)), issuer_of(&crl(b"abd", true)));
    // a long name (two length bytes)
    let long = vec![7u8; 300];
    assert_eq!(issuer_of(&crl(&long, true)), Some(&long[..]));
}

#[test]
fn a_crl_that_is_cut_short_has_no_readable_issuer() {
    let whole = crl(b"abc", true);
    for cut in 0..whole.len() {
        assert_eq!(issuer_of(&whole[..cut]), None, "cut at {cut}");
    }
    let mut trailing = whole.clone();
    trailing.push(0);
    assert_eq!(issuer_of(&trailing), None, "trailing data");
}

#[test]
fn lengths_that_lie_or_are_not_in_der_are_refused() {
    // a length longer than the input, a four-byte length of a gigabyte, an indefinite length, a length of five
    // bytes, a long form for a short length, a leading zero in the length, and a high tag number
    for bad in [
        &[0x30, 0x10, 0x30][..],
        &[0x30, 0x84, 0x40, 0, 0, 0, 0x30, 0],
        &[0x30, 0x80, 0x30, 0x00, 0x00, 0x00],
        &[0x30, 0x85, 0, 0, 0, 0, 1, 0],
        &[0x30, 0x81, 0x05, 1, 2, 3, 4, 5],
        &[0x30, 0x82, 0x00, 0x90, 0],
        &[0x1f, 0x01, 0x00],
        &[],
    ] {
        assert_eq!(tlv(bad), None, "{bad:02x?}");
        assert_eq!(issuer_of(bad), None);
    }
    // a leading zero in a length that otherwise fits the input (0x0080 in two bytes, with all 128 content bytes present)
    let mut padded = vec![0x30, 0x82, 0x00, 0x80];
    padded.extend([0u8; 128]);
    assert_eq!(tlv(&padded), None);
    let mut minimal = vec![0x30, 0x81, 0x80];
    minimal.extend([0u8; 128]);
    assert!(tlv(&minimal).is_some());
    // the maximum a usize can hold in four bytes is still refused, not wrapped
    assert_eq!(tlv(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]), None);
}

#[test]
fn deep_nesting_costs_nothing_because_nothing_recurses() {
    let deep = [0x30u8, 0x82, 0xff, 0xff].repeat(50_000);
    let started = Instant::now();
    assert_eq!(issuer_of(&deep), None);
    assert!(started.elapsed() < Duration::from_secs(1));
    // well-formed outer elements around a body of nothing but nested openers: read as flat bytes, never descended into
    let body = vec![0x30u8; 65_000];
    let wrapped = el(0x30, &el(0x30, &body));
    let started = Instant::now();
    let _ = issuer_of(&wrapped);
    assert!(started.elapsed() < Duration::from_secs(1));
}

// ---------------------------------------------------------------------------------------------------------------
// the validity times (#1614)

const UTC: u8 = 0x17;
const GEN: u8 = 0x18;

fn utc(text: &[u8]) -> Vec<u8> {
    el(UTC, text)
}

fn gen_time(text: &[u8]) -> Vec<u8> {
    el(GEN, text)
}

#[test]
fn valid_times_are_read_to_the_second() {
    for (tag, text, want) in [
        (UTC, &b"700101000000Z"[..], 0),
        (UTC, b"700101000001Z", 1),
        (UTC, b"700102000000Z", 86_400),
        (UTC, b"000101000000Z", 946_684_800),
        (UTC, b"491231235959Z", 2_524_607_999),
        (GEN, b"19700101000000Z", 0),
        (GEN, b"20500101000000Z", 2_524_608_000),
        // a leap day, in a year divisible by four and in one divisible by 400
        (UTC, b"240229120000Z", 1_709_208_000),
        (GEN, b"20000229000000Z", 951_782_400),
    ] {
        assert_eq!(
            time_to_unix(tag, text),
            Some(want),
            "{}",
            String::from_utf8_lossy(text)
        );
    }
}

#[test]
fn a_utctime_year_of_fifty_or_more_is_nineteen_hundreds_and_below_fifty_is_two_thousands() {
    // RFC 5280: YY >= 50 is 19YY, else 20YY
    let year = |yy: &[u8]| {
        let mut text = yy.to_vec();
        text.extend_from_slice(b"0101000000Z");
        time_to_unix(UTC, &text).unwrap()
    };
    assert_eq!(year(b"49"), 2_493_072_000);
    assert_eq!(year(b"50"), -631_152_000);
    assert_eq!(year(b"99"), 915_148_800);
    assert_eq!(year(b"00"), 946_684_800);
    assert!(
        year(b"49") > year(b"50"),
        "the pivot is a jump back a century"
    );
}

#[test]
fn times_that_break_the_strict_rules_are_refused() {
    for (tag, text) in [
        // wrong lengths
        (UTC, &b""[..]),
        (UTC, b"700101000000"),
        (UTC, b"7001010000Z"),
        (UTC, b"70010100000000Z"),
        (GEN, b"1970010100000Z"),
        (GEN, b"700101000000Z"),
        (UTC, b"19700101000000Z"),
        (GEN, b"197001010000000Z"),
        // no Z, an offset in its place, lower case z
        (UTC, b"7001010000000"),
        (UTC, b"700101000000z"),
        (UTC, b"70010100+0100"),
        (GEN, b"19700101000000+"),
        (GEN, b"19700101000000z"),
        // fractional seconds
        (GEN, b"19700101000000.Z"),
        (GEN, b"1970010100000.5Z"),
        // not digits: a sign, a space, a letter, a NUL, a high byte
        (UTC, b"+00101000000Z"),
        (UTC, b"70 101000000Z"),
        (UTC, b"7001010000a0Z"),
        (UTC, b"700101000\x000Z"),
        (UTC, b"\xff00101000000Z"),
        // month 0 and 13, day 0 and 32, a day the month does not have, a leap day in a year that has none
        (UTC, b"700001000000Z"),
        (UTC, b"701301000000Z"),
        (UTC, b"700100000000Z"),
        (UTC, b"700132000000Z"),
        (UTC, b"700431000000Z"),
        (UTC, b"700230000000Z"),
        (UTC, b"230229120000Z"),
        (GEN, b"19000229000000Z"),
        // hour 24, minute 60, second 60 (no leap second)
        (UTC, b"700101240000Z"),
        (UTC, b"700101006000Z"),
        (UTC, b"700101000060Z"),
        // not a time tag
        (0x13, b"700101000000Z"),
        (0x04, b"700101000000Z"),
    ] {
        assert_eq!(
            time_to_unix(tag, text),
            None,
            "tag {tag:#x} {}",
            String::from_utf8_lossy(text)
        );
    }
}

#[test]
fn the_next_update_is_read_and_a_missing_one_is_never() {
    let name = b"abc";
    let this = utc(b"700101000000Z");
    let with = |next: Option<Vec<u8>>, tail: &[u8]| {
        next_update_of(&crl_with(name, true, this.clone(), next, tail))
    };
    assert_eq!(
        with(Some(utc(b"700102000000Z")), &[]),
        Some(NextUpdate::At(86_400))
    );
    assert_eq!(
        with(Some(gen_time(b"20500101000000Z")), &[]),
        Some(NextUpdate::At(2_524_608_000))
    );
    // nothing after thisUpdate, then the two things that may follow a missing nextUpdate
    assert_eq!(with(None, &[]), Some(NextUpdate::Never));
    assert_eq!(with(None, &el(0x30, &[])), Some(NextUpdate::Never));
    assert_eq!(with(None, &el(0xa0, &[])), Some(NextUpdate::Never));
    // a real nextUpdate with a revoked list and extensions after it
    let tail = [el(0x30, &[]), el(0xa0, &[])].concat();
    assert_eq!(
        with(Some(utc(b"700102000000Z")), &tail),
        Some(NextUpdate::At(86_400))
    );
    // junk in the place of nextUpdate or after the times is not a CRL
    assert_eq!(with(None, &el(0x02, &[1])), None);
    assert_eq!(with(None, &[0x30]), None);
    assert_eq!(
        with(Some(utc(b"700102000000Z")), &[0x05]),
        Some(NextUpdate::At(86_400))
    );
}

#[test]
fn a_malformed_this_update_or_next_update_is_not_a_crl() {
    let good = utc(b"700101000000Z");
    let bad = [
        utc(b"701301000000Z"),
        utc(b"7001010000Z"),
        gen_time(b"19700101000000.0Z"),
        el(0x04, b"700101000000Z"),
        el(0x17, &[]),
    ];
    for b in &bad {
        assert_eq!(
            next_update_of(&crl_with(b"n", true, good.clone(), Some(b.clone()), &[])),
            None
        );
        assert_eq!(
            next_update_of(&crl_with(b"n", true, b.clone(), Some(good.clone()), &[])),
            None
        );
        assert_eq!(
            next_update_of(&crl_with(b"n", false, b.clone(), None, &[])),
            None
        );
    }
    // a CRL whose thisUpdate is missing altogether (a revoked-list SEQUENCE in its place)
    assert_eq!(
        next_update_of(&crl_with(b"n", true, el(0x30, &[]), None, &[])),
        None
    );
}

#[test]
fn a_crl_cut_short_has_no_readable_next_update_at_any_length() {
    let whole = crl_with(
        b"abc",
        true,
        utc(b"700101000000Z"),
        Some(gen_time(b"20500101000000Z")),
        &el(0x30, &[]),
    );
    assert!(next_update_of(&whole).is_some());
    for cut in 0..whole.len() {
        assert_eq!(next_update_of(&whole[..cut]), None, "cut at {cut}");
    }
}

#[test]
fn lying_and_indefinite_lengths_inside_the_times_are_refused() {
    let good = utc(b"700101000000Z");
    // a nextUpdate whose length says 13 but whose bytes are fewer, one that is indefinite, one in long form for a short length
    for next in [
        vec![UTC, 13, b'7', b'0'],
        vec![UTC, 0x80, 0, 0],
        vec![
            UTC, 0x81, 13, b'7', b'0', b'0', b'1', b'0', b'1', b'0', b'0', b'0', b'0', b'0', b'0',
            b'Z',
        ],
    ] {
        let mut tbs = el(0x30, &el(0x06, &[0x2a]));
        tbs.extend(el(0x30, b"n"));
        tbs.extend(good.clone());
        tbs.extend(next);
        let list = el(0x30, &tbs);
        assert_eq!(next_update_of(&el(0x30, &list)), None);
    }
}

#[test]
fn bounded_random_input_never_panics_and_never_reads_past_what_it_was_given() {
    // a deterministic generator (xorshift64), a few thousand cases: random bytes, and valid CRLs with a few bytes flipped,
    // cut or lengthened; the only assertion is that nothing panics, loops or allocates much (the functions take a slice
    // and return slices of it)
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let seeds = [
        crl_with(
            b"abc",
            true,
            utc(b"700101000000Z"),
            Some(utc(b"700102000000Z")),
            &el(0x30, &[]),
        ),
        crl_with(
            b"abc",
            false,
            utc(b"700101000000Z"),
            None,
            &el(0xa0, &[1, 2, 3]),
        ),
        crl_with(
            &[7u8; 300],
            true,
            gen_time(b"20500101000000Z"),
            Some(gen_time(b"20500101000000Z")),
            &[],
        ),
    ];
    let started = Instant::now();
    let mut decoded = 0usize;
    for case in 0..6000u32 {
        let input: Vec<u8> = if case % 3 == 0 {
            let len = (next() % 80) as usize;
            (0..len).map(|_| next() as u8).collect()
        } else {
            let mut v = seeds[(next() % 3) as usize].clone();
            for _ in 0..=(next() % 4) {
                let at = (next() as usize) % v.len();
                match next() % 4 {
                    0 => v[at] = next() as u8,
                    1 => v.truncate(at),
                    2 => v.insert(at, next() as u8),
                    _ => v[at] ^= 1 << (next() % 8),
                }
                if v.is_empty() {
                    break;
                }
            }
            v
        };
        let _ = tlv(&input);
        let _ = issuer_of(&input);
        if next_update_of(&input).is_some() {
            decoded += 1;
        }
        // the times themselves, from the same bytes, under both tags
        let _ = time_to_unix(UTC, &input);
        let _ = time_to_unix(GEN, &input);
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    // some mutated inputs still decode, so the generator does reach the interesting paths
    assert!(decoded > 100, "only {decoded} cases decoded");
}

#[test]
fn nesting_inside_the_validity_costs_nothing() {
    let body = [0x30u8, 0x82, 0xff, 0xff].repeat(40_000);
    let started = Instant::now();
    assert_eq!(next_update_of(&body), None);
    let wrapped = crl_with(b"n", true, utc(b"700101000000Z"), None, &body);
    let _ = next_update_of(&wrapped);
    assert!(started.elapsed() < Duration::from_secs(1));
}
