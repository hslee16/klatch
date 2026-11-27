//! The signature sanity check of a CRL against a listed intermediate certificate (#1648, refs #1623).
//!
//! A CRL whose issuer NAME matches a certificate of the client intermediates file, but whose signature was not made by
//! that certificate's key, passes the name match and then refuses every client of that issuer at handshake time (the
//! verifier checks the signature against the chain). This turns that into a refusal at start, with the CRL's number in the
//! message. It is a sanity check of the file, **never a trust decision**: the certificate is still no trust anchor, nothing
//! here widens what the TLS layer accepts, and a result of "cannot tell" always passes (so a signature algorithm this
//! module does not know, or a CRL it cannot read, is left to the verifier exactly as before). The check can only turn a
//! start that would have locked an issuer out into a refusal.

use crate::der::{issuer_of, tlv};
use crate::tls::Intermediate;
use anyhow::{Result, bail};
use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use rustls_pki_types::CertificateRevocationListDer;
use std::path::Path;

const SEQUENCE: u8 = 0x30;
const BIT_STRING: u8 = 0x03;
const OID: u8 = 0x06;

const ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const ECDSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03];
const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const ED25519: &[u8] = &[0x2b, 0x65, 0x70];
const RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const RSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const RSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c];
const RSA_SHA512: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d];

/// What the check could say about one CRL and one certificate key.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Valid,
    Invalid,
    /// An algorithm or shape not known here: the verifier decides, as before.
    Unknown,
}

/// The bytes of one OID element at the start of `input`, and what follows it.
fn oid(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (tag, content, rest) = tlv(input)?;
    (tag == OID).then_some((content, rest))
}

/// The content of a BIT STRING with no unused bits (the shape of a signature or a key).
fn bits(input: &[u8]) -> Option<&[u8]> {
    let (tag, content, _) = tlv(input)?;
    match content.split_first() {
        Some((0, rest)) if tag == BIT_STRING => Some(rest),
        _ => None,
    }
}

fn judge(crl: &[u8], spki: &[u8]) -> Verdict {
    let Some(parts) = crl_parts(crl) else {
        return Verdict::Unknown;
    };
    let (signed, sig_alg, sig) = parts;
    let Some((key_alg, key)) = key_parts(spki) else {
        return Verdict::Unknown;
    };
    let Some((sig_oid, _)) = oid(sig_alg) else {
        return Verdict::Unknown;
    };
    let Some((key_oid, key_params)) = oid(key_alg) else {
        return Verdict::Unknown;
    };
    let curve = oid(key_params).map(|(c, _)| c);
    let alg: &'static dyn VerificationAlgorithm = match (sig_oid, key_oid, curve) {
        (ECDSA_SHA256, EC_PUBLIC_KEY, Some(P256)) => &signature::ECDSA_P256_SHA256_ASN1,
        (ECDSA_SHA384, EC_PUBLIC_KEY, Some(P384)) => &signature::ECDSA_P384_SHA384_ASN1,
        (ED25519, ED25519, _) => &signature::ED25519,
        (RSA_SHA256, RSA_ENCRYPTION, _) => &signature::RSA_PKCS1_2048_8192_SHA256,
        (RSA_SHA384, RSA_ENCRYPTION, _) => &signature::RSA_PKCS1_2048_8192_SHA384,
        (RSA_SHA512, RSA_ENCRYPTION, _) => &signature::RSA_PKCS1_2048_8192_SHA512,
        _ => return Verdict::Unknown,
    };
    match UnparsedPublicKey::new(alg, key).verify(signed, sig) {
        Ok(()) => Verdict::Valid,
        Err(_) => Verdict::Invalid,
    }
}

/// A CRL's `tbsCertList` as the signed bytes (header included), its `signatureAlgorithm` content and its signature bits.
fn crl_parts(crl: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let (tag, body, after) = tlv(crl)?;
    if tag != SEQUENCE || !after.is_empty() {
        return None;
    }
    let (tag, _, rest) = tlv(body)?;
    if tag != SEQUENCE {
        return None;
    }
    let signed = &body[..body.len() - rest.len()];
    let (tag, alg, rest) = tlv(rest)?;
    if tag != SEQUENCE {
        return None;
    }
    Some((signed, alg, bits(rest)?))
}

/// The algorithm identifier content and the key bits of a `subjectPublicKeyInfo` content.
fn key_parts(spki: &[u8]) -> Option<(&[u8], &[u8])> {
    let (tag, alg, rest) = tlv(spki)?;
    (tag == SEQUENCE).then_some(())?;
    Some((alg, bits(rest)?))
}

/// Refuse a CRL whose issuer is a listed intermediate (and no CA of the client CA file, whose key is the verifier's to
/// judge) when no listed certificate of that name verifies its signature. `path` and the CRL's number are named, never a
/// subject or a serial.
pub fn check(
    crls: &[CertificateRevocationListDer<'_>],
    ca_subjects: &[&[u8]],
    listed: &[Intermediate],
    path: &Path,
) -> Result<()> {
    for (i, crl) in crls.iter().enumerate() {
        let Some(issuer) = issuer_of(crl.as_ref()) else {
            continue;
        };
        if ca_subjects.contains(&issuer) {
            continue;
        }
        let verdicts: Vec<Verdict> = listed
            .iter()
            .filter(|c| c.subject == issuer)
            .map(|c| judge(crl.as_ref(), &c.spki))
            .collect();
        if !verdicts.is_empty() && verdicts.iter().all(|v| *v == Verdict::Invalid) {
            bail!(
                "the client CRL file {} holds a CRL (number {}) whose signature does not verify against the certificate listed for its issuer in the client intermediates file: it would refuse every client of that issuer; replace the CRL, or the certificate in the intermediates file",
                path.display(),
                i + 1
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::der::subject_and_key_of;
    use rcgen::{
        BasicConstraints, CertificateParams, CertificateRevocationListParams, DnType, IsCa, Issuer,
        KeyIdMethod, KeyPair, KeyUsagePurpose, SerialNumber, SignatureAlgorithm,
    };
    use std::time::{Duration, SystemTime};

    fn ca(alg: &'static SignatureAlgorithm) -> (CertificateParams, KeyPair, Vec<u8>) {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.distinguished_name.push(DnType::CommonName, "sig-ca");
        let key = KeyPair::generate_for(alg).unwrap();
        let cert = params.self_signed(&key).unwrap();
        (params, key, cert.der().to_vec())
    }

    fn crl(params: &CertificateParams, signer: &KeyPair) -> Vec<u8> {
        let now = SystemTime::now();
        let list = CertificateRevocationListParams {
            this_update: now.into(),
            next_update: (now + Duration::from_secs(86_400)).into(),
            crl_number: SerialNumber::from(1u64),
            issuing_distribution_point: None,
            revoked_certs: Vec::new(),
            key_identifier_method: KeyIdMethod::Sha256,
        };
        let issuer = Issuer::new(params.clone(), signer);
        list.signed_by(&issuer).unwrap().der().to_vec()
    }

    /// A signature verifies for each key type this module knows, fails for another key of the same type and for a changed
    /// byte of the signed part (the three mutants of a verifier that checks nothing, or too little).
    #[test]
    fn a_signature_is_valid_only_for_the_key_that_made_it_for_each_known_algorithm() {
        for alg in [
            &rcgen::PKCS_ECDSA_P256_SHA256,
            &rcgen::PKCS_ECDSA_P384_SHA384,
            &rcgen::PKCS_ED25519,
        ] {
            let (params, key, cert) = ca(alg);
            let (_, spki) = subject_and_key_of(&cert).unwrap();
            let good = crl(&params, &key);
            assert_eq!(judge(&good, spki), Verdict::Valid, "{alg:?}");
            let forged = crl(&params, &KeyPair::generate_for(alg).unwrap());
            assert_eq!(judge(&forged, spki), Verdict::Invalid, "{alg:?}");
            // one byte of the signed part changed (the serial of the CRL, inside tbsCertList)
            let mut tampered = good.clone();
            let at = tampered.len() / 3;
            tampered[at] ^= 0x01;
            assert_ne!(judge(&tampered, spki), Verdict::Valid, "{alg:?}");
        }
    }

    #[test]
    fn what_this_module_does_not_know_or_cannot_read_is_left_to_the_verifier() {
        let (params, key, cert) = ca(&rcgen::PKCS_ECDSA_P256_SHA256);
        let (_, spki) = subject_and_key_of(&cert).unwrap();
        let good = crl(&params, &key);
        // a key of another type than the signature algorithm names: not judged
        let (_, _, other) = ca(&rcgen::PKCS_ED25519);
        let (_, other_spki) = subject_and_key_of(&other).unwrap();
        assert_eq!(judge(&good, other_spki), Verdict::Unknown);
        // garbage for the CRL or for the key info
        assert_eq!(judge(b"\x30\x00", spki), Verdict::Unknown);
        assert_eq!(judge(&good, b"\x30\x00"), Verdict::Unknown);
        assert_eq!(judge(&[], spki), Verdict::Unknown);
    }

    fn listed(cert: &[u8]) -> Vec<Intermediate> {
        let (subject, spki) = subject_and_key_of(cert).unwrap();
        vec![Intermediate {
            subject: subject.to_vec(),
            spki: spki.to_vec(),
        }]
    }

    #[test]
    fn check_refuses_a_forged_crl_of_a_listed_issuer_and_names_its_number_only() {
        let (params, key, cert) = ca(&rcgen::PKCS_ECDSA_P256_SHA256);
        let inters = listed(&cert);
        let good = CertificateRevocationListDer::from(crl(&params, &key));
        let forged =
            CertificateRevocationListDer::from(crl(&params, &KeyPair::generate().unwrap()));
        let path = Path::new("/d/crl.pem");
        assert!(check(std::slice::from_ref(&good), &[], &inters, path).is_ok());
        let err = check(&[good.clone(), forged.clone()], &[], &inters, path).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("(number 2)") && text.contains("/d/crl.pem"),
            "{text}"
        );
        assert!(!text.contains("sig-ca"), "{text}");
        // an issuer that is also a CA of the CA file is the verifier's to judge
        let (subject, _) = subject_and_key_of(&cert).unwrap();
        assert!(check(std::slice::from_ref(&forged), &[subject], &inters, path).is_ok());
        // an issuer that is not listed is not judged here
        assert!(check(&[forged], &[], &[], path).is_ok());
    }

    /// The intermediates file is held to the owner and mode rule of the CA file, with its own words about what a writer
    /// could do (it changes which CRL issuers pass the start check; it does not choose who may connect) (#1648). Kills the
    /// mutant that reads it without the owner and mode checks.
    #[test]
    fn the_intermediates_file_needs_its_owner_and_a_mode_and_says_what_a_writer_could_do() {
        use crate::tls::{euid, read_intermediates_as, read_pem};
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let path =
            std::path::PathBuf::from(format!("/tmp/kt-unit-{}-inters.pem", std::process::id()));
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let pem = params
            .self_signed(&KeyPair::generate().unwrap())
            .unwrap()
            .pem();
        std::fs::write(&path, pem).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_intermediates_as(&path, euid()).unwrap().len(), 1);
        // a server running as someone who is neither the owner nor root refuses the file
        let owner = std::fs::metadata(&path).unwrap().uid();
        if owner != 0 {
            let err = read_intermediates_as(&path, owner + 1)
                .err()
                .unwrap()
                .to_string();
            assert!(
                err.contains("is not owned by the user running the server or by root"),
                "{err}"
            );
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
        let err = read_intermediates_as(&path, euid())
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.contains("writable by group or other (mode 0664)"),
            "{err}"
        );
        assert!(
            err.contains("changes which CRL issuers pass the start check"),
            "{err}"
        );
        assert!(!err.contains("chooses who may connect"), "{err}");
        // the CA file keeps its own words
        let err = read_pem(&path, "client CA file", true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("chooses who may connect"), "{err}");
        let _ = std::fs::remove_file(&path);
    }
}
