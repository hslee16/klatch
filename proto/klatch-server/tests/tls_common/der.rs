//! Just enough DER to cut a CRL apart and put it together again (tests of CRLs the verifier must refuse by their shape).
#![allow(dead_code)]

/// DER length bytes.
pub fn len_bytes(n: usize) -> Vec<u8> {
    match n {
        n if n < 0x80 => vec![n as u8],
        n if n < 0x100 => vec![0x81, n as u8],
        n => vec![0x82, (n >> 8) as u8, n as u8],
    }
}

/// One element: tag, content, and what follows it.
pub fn element(input: &[u8]) -> (u8, &[u8], &[u8]) {
    let (tag, first) = (input[0], input[1] as usize);
    let (len, at) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        (
            input[2..2 + n]
                .iter()
                .fold(0usize, |a, b| (a << 8) | *b as usize),
            2 + n,
        )
    };
    (tag, &input[at..at + len], &input[at + len..])
}

pub fn wrap(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(len_bytes(content.len()));
    out.extend_from_slice(content);
    out
}

pub fn pem_of(der: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = String::new();
    for chunk in der.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, b)| a | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                b64.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    let lines: Vec<&str> = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN X509 CRL-----\n{}\n-----END X509 CRL-----\n",
        lines.join("\n")
    )
}

/// The CRL of `pem` with its signature element replaced by bytes that are not a signature: the start-up read of the
/// issuer and the times (which come from the part before the signature) still accepts it, and the verifier does not.
pub fn with_junk_signature(pem: &str) -> String {
    use rustls::pki_types::{CertificateRevocationListDer, pem::PemObject};
    let der = CertificateRevocationListDer::from_pem_slice(pem.as_bytes()).unwrap();
    let (_, list, _) = element(der.as_ref());
    let (tbs_tag, tbs, after_tbs) = element(list);
    let (alg_tag, alg, _signature) = element(after_tbs);
    let mut kept = wrap(tbs_tag, tbs);
    kept.extend(wrap(alg_tag, alg));
    kept.extend(wrap(0x04, b"not a signature"));
    pem_of(&wrap(0x30, &kept))
}
