//! Helpers the harnesses of more than one area share.

/// Text of at most `N` characters drawn from `alphabet`, with a symbolic length. The alphabet is the characters a
/// function treats specially plus one ordinary letter, which is enough to reach every branch of a string function
/// that only looks at those characters. Built from a byte array and borrowed, not grown in a `String`, which costs
/// the solver far more than the function under test.
pub(super) fn text<const N: usize>(alphabet: &[u8]) -> &'static str {
    let mut bytes = [0u8; N];
    for b in bytes.iter_mut() {
        let i: usize = kani::any();
        kani::assume(i < alphabet.len());
        *b = alphabet[i];
    }
    let len: usize = kani::any();
    kani::assume(len <= N);
    let leaked: &'static [u8; N] = Box::leak(Box::new(bytes));
    std::str::from_utf8(&leaked[..len]).unwrap()
}

/// Up to `N` bytes from `alphabet`, as an array and a length.
pub(super) fn bytes_of<const N: usize>(alphabet: &[u8]) -> ([u8; N], usize) {
    let mut bytes = [0u8; N];
    for b in bytes.iter_mut() {
        let i: usize = kani::any();
        kani::assume(i < alphabet.len());
        *b = alphabet[i];
    }
    let len: usize = kani::any();
    kani::assume(len <= N);
    (bytes, len)
}

pub(super) fn no_dot_segments(b: &[u8]) -> bool {
    // a segment is `.` or `..` when it sits between `/`, or at either end, with nothing else in it
    let mut start = 0;
    for i in 0..=b.len() {
        if i == b.len() || b[i] == b'/' {
            let seg = &b[start..i];
            if seg == b"." || seg == b".." {
                return false;
            }
            start = i + 1;
        }
    }
    true
}
