//! Tests of the certificate subject reader (`der::subject_of`, #1623): the field it must read, and malformed input of
//! every kind (truncation at every length, lying, indefinite and non-minimal lengths, deep nesting, a bounded
//! deterministic random run) which must be `None` and never a panic.

use crate::der::{subject_of, tlv};
use crate::der_tests::el;
use std::time::{Duration, Instant};

const SEQ: u8 = 0x30;

fn alg() -> Vec<u8> {
    el(SEQ, &el(0x06, &[0x2a]))
}

/// A certificate's outer shape. `issuer`, `subject` are the Name contents; `version` adds the `[0]` element.
fn cert(issuer: &[u8], subject: &[u8], version: bool) -> Vec<u8> {
    cert_with(issuer, el(SEQ, subject), version)
}

/// [`cert`] with the subject element given whole (a test of a subject whose length is wrong).
fn cert_with(issuer: &[u8], subject_element: Vec<u8>, version: bool) -> Vec<u8> {
    let mut tbs = Vec::new();
    if version {
        tbs.extend(el(0xa0, &el(0x02, &[2])));
    }
    tbs.extend(el(0x02, &[0x2a]));
    tbs.extend(alg());
    tbs.extend(el(SEQ, issuer));
    tbs.extend(el(
        SEQ,
        &[el(0x17, b"700101000000Z"), el(0x17, b"700102000000Z")].concat(),
    ));
    tbs.extend(subject_element);
    tbs.extend(el(SEQ, &[]));
    let mut body = el(SEQ, &tbs);
    body.extend(alg());
    body.extend(el(0x03, &[0, 1]));
    el(SEQ, &body)
}

#[test]
fn the_subject_is_read_not_the_issuer_with_or_without_a_version() {
    for version in [true, false] {
        let c = cert(b"issuer-name", b"subject-name", version);
        assert_eq!(subject_of(&c), Some(&b"subject-name"[..]));
    }
    // the same bytes in both fields: still the second Name, and a long name (two length bytes) works
    let long = vec![7u8; 300];
    assert_eq!(subject_of(&cert(b"x", &long, true)), Some(&long[..]));
    // RFC 5280: a CA subject is a non-empty name; an empty one would let a CRL with an empty issuer pass
    assert_eq!(subject_of(&cert(b"", b"", true)), None);
    assert_eq!(subject_of(&cert(b"", b"", false)), None);
    assert_eq!(subject_of(&cert(b"x", b"", true)), None);
}

#[test]
fn a_certificate_cut_short_has_no_subject_at_any_length() {
    let whole = cert(b"issuer", b"subject", true);
    for cut in 0..whole.len() {
        assert_eq!(subject_of(&whole[..cut]), None, "cut at {cut}");
    }
    let mut trailing = whole.clone();
    trailing.push(0);
    assert_eq!(subject_of(&trailing), None, "trailing data");
}

#[test]
fn elements_out_of_place_are_not_a_certificate() {
    let ok = cert(b"i", b"s", true);
    assert!(subject_of(&ok).is_some());
    // each field's tag changed in turn: the walk must refuse a certificate whose fields are not where X.509 puts them
    let (_, body, _) = tlv(&ok).unwrap();
    let (_, tbs, _) = tlv(body).unwrap();
    let mut at = 0;
    let mut offsets = Vec::new();
    let mut rest = tbs;
    while !rest.is_empty() {
        let (_, _, next) = tlv(rest).unwrap();
        offsets.push(at);
        at += rest.len() - next.len();
        rest = next;
    }
    // offsets of the tbs fields inside the whole certificate: outer header + body header + tbs header
    let (_, _, after_tbs) = tlv(body).unwrap();
    let base = (ok.len() - body.len()) + (body.len() - after_tbs.len() - tbs.len());
    // the six fields up to the subject (what follows it is not read)
    for off in offsets.into_iter().take(6) {
        let mut bad = ok.clone();
        bad[base + off] = 0x04;
        assert_eq!(subject_of(&bad), None, "tag changed at tbs offset {off}");
    }
    // a signature algorithm or value that is missing or extra
    let tbs_only = el(SEQ, &el(SEQ, tbs));
    assert_eq!(subject_of(&tbs_only), None);
    let mut extra = tlv(&ok).unwrap().1.to_vec();
    extra.extend(el(0x04, &[1]));
    assert_eq!(subject_of(&el(SEQ, &extra)), None);
}

#[test]
fn lengths_that_lie_or_are_not_in_der_are_not_a_certificate() {
    let ok = cert(b"i", b"s", true);
    for bad in [
        // an indefinite length, a length past the input, a long form for a short length, a leading zero, a high tag
        [&[0x30u8, 0x80][..], &ok[2..], &[0, 0]].concat(),
        [&[0x30u8, 0x84, 0x40, 0, 0, 0][..], &ok[2..]].concat(),
        [&[0x30u8, 0x81, ok.len() as u8 - 2][..], &ok[2..]].concat(),
        [&[0x30u8, 0x82, 0x00, ok.len() as u8 - 2][..], &ok[2..]].concat(),
        [&[0x1fu8, 0x01][..], &ok[2..]].concat(),
        vec![],
        vec![0x30],
        vec![0x30, 0x00],
    ] {
        // (ok[2..] is only the same length in the short form, which a certificate this small has)
        if ok[1] < 0x80 {
            assert_eq!(subject_of(&bad), None, "{bad:02x?}");
        }
    }
    // a non-minimal, indefinite, lying or oversized length on the subject itself
    for subject in [
        vec![SEQ, 0x81, 0x01, b'a'],
        vec![SEQ, 0x80, b'a', 0, 0],
        vec![SEQ, 0x05, b'a'],
        vec![SEQ, 0x84, 0x7f, 0xff, 0xff, 0xff, b'a'],
        vec![SEQ, 0x82, 0x00, 0x01, b'a'],
    ] {
        assert_eq!(
            subject_of(&cert_with(b"i", subject.clone(), true)),
            None,
            "{subject:02x?}"
        );
    }
    // and the control: the same builder with a well formed subject element reads
    assert_eq!(
        subject_of(&cert_with(b"i", el(SEQ, b"a"), true)),
        Some(&b"a"[..])
    );
}

#[test]
fn deep_nesting_costs_nothing_because_nothing_recurses() {
    let body = [0x30u8, 0x82, 0xff, 0xff].repeat(40_000);
    let started = Instant::now();
    assert_eq!(subject_of(&body), None);
    let wrapped = el(SEQ, &el(SEQ, &body[..60_000.min(body.len())]));
    assert_eq!(subject_of(&wrapped), None);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn bounded_random_input_never_panics_and_still_reaches_the_interesting_paths() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let seeds = [
        cert(b"issuer", b"subject", true),
        cert(b"issuer", b"subject", false),
        cert(&[7u8; 300], &[9u8; 200], true),
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
        let got = subject_of(&input);
        if let Some(subject) = got {
            decoded += 1;
            // whatever it returns lies inside the input it was given
            let from = subject.as_ptr() as usize - input.as_ptr() as usize;
            assert!(from + subject.len() <= input.len());
        }
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(decoded > 100, "only {decoded} cases decoded");
}
