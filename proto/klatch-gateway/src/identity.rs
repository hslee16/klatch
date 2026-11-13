//! Who is calling, decided at the transport edge (DESIGN section 20 rule 11): a transport gathers
//! `PeerEvidence` about the peer and an `Identity` maps it to the `Principal` the gateway takes. A client
//! never chooses its own name; the caller of the gateway is whoever the evidence supports.

use klatch_core::{Error, Principal, Result, valid_name};

/// What a transport knows about its peer. New kinds of evidence (a session found from the process tree, an
/// SSH key, a certificate) are added as variants, which is why the enum is not exhaustive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PeerEvidence {
    /// Nothing is known about the peer.
    None,
    /// A name the peer typed (an environment variable or a flag). It proves nothing.
    DeclaredName(String),
}

/// Maps evidence to a caller. Implementations must never return a verified principal on evidence that
/// does not prove who the peer is.
pub trait Identity {
    fn resolve(&self, evidence: &PeerEvidence) -> Result<Principal>;
}

/// The prototype's identity: believes the typed name, so the caller is always unattributed (R23). It is
/// forgeable by design and exists so everything else can be built and tested behind the trait.
pub struct DeclaredName;

/// What to tell a human whose name was missing or refused. It never repeats the name that was typed.
const NAME_HINT: &str = "set KLATCH_NAME (or --as-member) to 1-40 chars of [A-Za-z0-9._-]";

impl Identity for DeclaredName {
    fn resolve(&self, evidence: &PeerEvidence) -> Result<Principal> {
        match evidence {
            PeerEvidence::DeclaredName(name) if valid_name(name) => {
                Ok(Principal::unattributed(name))
            }
            PeerEvidence::DeclaredName(_) => Err(Error::Invalid(format!(
                "the member name is not valid: {NAME_HINT}"
            ))),
            _ => Err(Error::Invalid(format!("no member name given: {NAME_HINT}"))),
        }
    }
}

/// An identity that returns one fixed principal for any evidence. For tests and for callers that already
/// know who they are acting as.
pub struct FixedIdentity(pub Principal);

impl Identity for FixedIdentity {
    fn resolve(&self, _evidence: &PeerEvidence) -> Result<Principal> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klatch_core::{Attribution, Error};

    fn declared(name: &str) -> PeerEvidence {
        PeerEvidence::DeclaredName(name.into())
    }

    #[test]
    fn a_declared_name_resolves_to_an_unattributed_principal_with_that_name() {
        let p = DeclaredName.resolve(&declared("alice")).unwrap();
        assert_eq!(p.member, "alice");
        assert_eq!(p.attribution, Attribution::Unattributed);
    }

    #[test]
    fn a_typed_name_can_never_be_verified_whatever_it_says() {
        for name in [
            "root",
            "admin",
            "verified",
            "klatch",
            "gateway",
            "carol",
            "Verified-by-the-human",
        ] {
            let p = DeclaredName.resolve(&declared(name)).unwrap();
            assert_eq!(p.attribution, Attribution::Unattributed, "{name}");
        }
    }

    #[test]
    fn invalid_names_are_refused_with_a_hint_that_does_not_echo_the_name() {
        let long = "x".repeat(41);
        for name in [
            "",
            " ",
            "a b",
            "a\nb",
            "a/b",
            " alice",
            "alice ",
            long.as_str(),
        ] {
            let err = DeclaredName.resolve(&declared(name)).unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{name:?}");
            let text = err.to_string();
            assert!(text.contains("KLATCH_NAME"), "{text}");
            if name.len() > 3 {
                assert!(
                    !text.contains(name),
                    "the error must not echo the typed name"
                );
            }
        }
    }

    #[test]
    fn no_evidence_means_no_identity() {
        let err = DeclaredName.resolve(&PeerEvidence::None).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
        assert!(err.to_string().contains("KLATCH_NAME"));
    }

    #[test]
    fn the_forgeable_identity_works_through_a_trait_object() {
        let id: &dyn Identity = &DeclaredName;
        assert_eq!(id.resolve(&declared("bob")).unwrap().member, "bob");
    }

    #[test]
    fn a_fixed_identity_ignores_the_evidence_and_can_be_verified_for_tests() {
        let id = FixedIdentity(Principal::verified("carol"));
        for ev in [PeerEvidence::None, declared("anyone-else")] {
            let p = id.resolve(&ev).unwrap();
            assert_eq!(
                (p.member.as_str(), p.attribution),
                ("carol", Attribution::Verified)
            );
        }
    }

    #[test]
    fn evidence_debug_output_is_available_for_diagnostics() {
        assert!(format!("{:?}", declared("alice")).contains("DeclaredName"));
    }
}
