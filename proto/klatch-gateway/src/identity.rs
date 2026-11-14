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
    /// A session of a coding-agent harness found from the process tree and the harness registry (same-user
    /// evidence: anything running as the same user can read the registry, so it is not proof).
    Session {
        session_id: String,
        name: Option<String>,
    },
    /// A credential the gateway issued, already checked against the registry (token, expiry, revocation) by the
    /// caller. This is the one kind of evidence that yields a verified principal.
    Credential { member: String },
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

/// The identity for a local process: believes a typed name or a session found from the process tree. Both are
/// same-user evidence, so the caller is always unattributed until a stronger source exists.
pub struct LocalIdentity;

/// A character a member name may carry as it is.
fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.')
}

/// Turn a session's own name into a valid member name: every character outside the allowed set becomes `-`,
/// the ends are trimmed and the result is cut to 40 characters. Empty results are `None`.
fn member_name_from(raw: &str) -> Option<String> {
    let mapped: String = raw
        .chars()
        .map(|c| if name_char(c) { c } else { '-' })
        .collect();
    let cut: String = mapped.trim_matches('-').chars().take(40).collect();
    let cut = cut.trim_matches('-').to_string();
    valid_name(&cut).then_some(cut)
}

impl Identity for LocalIdentity {
    fn resolve(&self, evidence: &PeerEvidence) -> Result<Principal> {
        match evidence {
            PeerEvidence::Session { session_id, name } => {
                let member = name
                    .as_deref()
                    .and_then(member_name_from)
                    .or_else(|| {
                        let short: String = session_id.chars().take(8).collect();
                        member_name_from(&format!("session-{short}")).filter(|_| !short.is_empty())
                    })
                    .ok_or_else(|| Error::Invalid("the session has no usable name or id".into()))?;
                Ok(Principal::unattributed(&member))
            }
            PeerEvidence::Credential { member } if valid_name(member) => {
                Ok(Principal::verified(member))
            }
            PeerEvidence::Credential { .. } => Err(Error::Invalid(
                "the credential names no valid member".into(),
            )),
            other => DeclaredName.resolve(other),
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
    fn only_a_checked_credential_resolves_to_a_verified_principal() {
        let p = LocalIdentity
            .resolve(&PeerEvidence::Credential {
                member: "w-1".into(),
            })
            .unwrap();
        assert_eq!(
            (p.member.as_str(), p.attribution),
            ("w-1", Attribution::Verified)
        );
        assert!(
            LocalIdentity
                .resolve(&PeerEvidence::Credential {
                    member: "bad name".into()
                })
                .is_err()
        );
        // a typed name and a session stay unattributed
        assert_eq!(
            LocalIdentity.resolve(&declared("w-1")).unwrap().attribution,
            Attribution::Unattributed
        );
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

    fn session(id: &str, name: Option<&str>) -> PeerEvidence {
        PeerEvidence::Session {
            session_id: id.into(),
            name: name.map(Into::into),
        }
    }

    #[test]
    fn a_session_resolves_to_its_name_and_is_never_verified() {
        let p = LocalIdentity
            .resolve(&session("abc-123", Some("klatch-1b")))
            .unwrap();
        assert_eq!(p.member, "klatch-1b");
        assert_eq!(p.attribution, Attribution::Unattributed);
    }

    #[test]
    fn a_session_name_is_made_into_a_valid_member_name() {
        let cases = [
            ("My Session!", "My-Session"),
            ("a/b c", "a-b-c"),
            ("line\nbreak", "line-break"),
            ("  spaced  ", "spaced"),
            ("ünï", "n"),
        ];
        for (raw, want) in cases {
            let p = LocalIdentity
                .resolve(&session("id12345678", Some(raw)))
                .unwrap();
            assert_eq!(p.member, want, "{raw}");
            assert!(valid_name(&p.member));
        }
        let long = "x".repeat(60);
        let p = LocalIdentity.resolve(&session("i", Some(&long))).unwrap();
        assert_eq!(p.member.len(), 40);
    }

    #[test]
    fn a_session_without_a_usable_name_is_named_from_its_id() {
        for name in [None, Some(""), Some("!!!"), Some("   ")] {
            let p = LocalIdentity
                .resolve(&session("0123456789abcdef", name))
                .unwrap();
            assert_eq!(p.member, "session-01234567", "{name:?}");
        }
    }

    #[test]
    fn a_session_with_no_name_and_no_id_is_refused() {
        let err = LocalIdentity.resolve(&session("", None)).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn the_session_id_is_never_the_whole_member_name_so_it_cannot_leak_into_messages() {
        let id = "0123456789abcdef-secretish";
        let p = LocalIdentity.resolve(&session(id, None)).unwrap();
        assert!(!p.member.contains("secretish"));
    }

    #[test]
    fn local_identity_still_handles_a_typed_name_and_no_evidence() {
        assert_eq!(
            LocalIdentity.resolve(&declared("alice")).unwrap().member,
            "alice"
        );
        assert!(LocalIdentity.resolve(&PeerEvidence::None).is_err());
    }

    #[test]
    fn the_declared_name_identity_does_not_believe_a_session() {
        let err = DeclaredName
            .resolve(&session("abc", Some("x")))
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn evidence_debug_output_is_available_for_diagnostics() {
        assert!(format!("{:?}", declared("alice")).contains("DeclaredName"));
    }
}
