//! What a listener must satisfy before it starts or accepts a peer (#157, R33). No network transport exists yet
//! (tickets #107 to #113); each one calls these two functions, and a transport that does not is a bug.
//!
//! Local stdio and Unix sockets are not covered here: they rely on operating-system credentials (DESIGN section 32
//! item 6).

use std::net::{IpAddr, SocketAddr};

/// How a listener protects its connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// No encryption. Only for loopback, and only with a bearer token required of every request.
    Plaintext,
    /// TLS with a certificate required of the client as well as the server.
    MutualTls,
    /// SSH, with the host key verified by the client and a key required of it.
    Ssh,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A plaintext listener off loopback.
    PlaintextOffLoopback(SocketAddr),
    /// A plaintext listener on loopback that asks for no token.
    PlaintextWithoutToken,
    /// A mutual TLS listener with no CA file for client certificates: it would have nobody to check a client against.
    MutualTlsWithoutClientCa,
    /// A mutual TLS listener off loopback that the operator did not ask, in words, to run there.
    OffLoopbackNotAllowed(SocketAddr),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PlaintextOffLoopback(a) => write!(
                f,
                "refusing to listen on {a} without encryption: a listener that is not on loopback must use mutual TLS or SSH"
            ),
            Self::PlaintextWithoutToken => f.write_str(
                "refusing a plaintext listener that does not require a token: any local user could connect to it",
            ),
            Self::MutualTlsWithoutClientCa => f.write_str(
                "refusing a TLS listener without a client CA: mutual TLS needs a CA file whose certificates may connect (--tls-client-ca)",
            ),
            Self::OffLoopbackNotAllowed(a) => write!(
                f,
                "refusing to listen on {a}: a mutual TLS listener is loopback only unless --allow-off-loopback is given"
            ),
        }
    }
}
impl std::error::Error for Refusal {}

/// Whether the address is loopback, including an IPv4 loopback written as an IPv6 mapped address. An unspecified
/// address (`0.0.0.0`, `::`) listens on every interface and is not loopback.
pub fn is_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// Whether a listener may bind `addr` with `security`. `requires_token` says every request must carry a bearer
/// token (the only protection a plaintext loopback listener has against other local users).
pub fn check_bind(
    addr: &SocketAddr,
    security: Security,
    requires_token: bool,
) -> Result<(), Refusal> {
    match security {
        Security::MutualTls | Security::Ssh => Ok(()),
        Security::Plaintext if !is_loopback(&addr.ip()) => {
            Err(Refusal::PlaintextOffLoopback(*addr))
        }
        Security::Plaintext if !requires_token => Err(Refusal::PlaintextWithoutToken),
        Security::Plaintext => Ok(()),
    }
}

/// What the operator set up for a mutual TLS listener (#108).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TlsBind {
    /// A CA file for client certificates is configured. Without one the listener does not start.
    pub client_ca_configured: bool,
    /// The operator asked, explicitly, for an address that is not loopback. Off by default.
    pub allow_off_loopback: bool,
}

/// Whether a mutual TLS listener may bind `addr` (#108). It needs a client CA wherever it listens, and it listens off
/// loopback only when the operator said so. This is the only way a klatch listener may leave loopback: `check_bind`
/// with `Security::Plaintext` still refuses every address that is not loopback, and nothing here changes that.
pub fn check_mutual_tls_bind(addr: &SocketAddr, tls: TlsBind) -> Result<(), Refusal> {
    if !tls.client_ca_configured {
        return Err(Refusal::MutualTlsWithoutClientCa);
    }
    if !is_loopback(&addr.ip()) && !tls.allow_off_loopback {
        return Err(Refusal::OffLoopbackNotAllowed(*addr));
    }
    Ok(())
}

/// What a connecting peer proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    /// Nothing: no certificate, no key, no token.
    Anonymous,
    /// A bearer token that matched.
    Token,
    /// A client certificate; `chain_verified` is whether it was checked against the trusted roots.
    Certificate { chain_verified: bool },
    /// An SSH key; `host_key_verified` is whether the server's host key was checked by the client (a client that
    /// skips that check can be talking to anyone).
    SshKey { host_key_verified: bool },
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeerRefusal {
    Anonymous,
    CertificateNotVerified,
    HostKeyNotVerified,
    /// A token is not enough on an encrypted transport.
    TokenOnEncrypted,
    /// A certificate or key offered to a transport of another kind.
    WrongKind,
}

/// Whether `peer` may be served over `security`. Certificates and host keys must be verified, never skipped; a
/// plaintext listener takes only a token; an encrypted one takes only what it asks for.
pub fn check_peer(security: Security, peer: &Peer) -> Result<(), PeerRefusal> {
    match (security, peer) {
        (_, Peer::Anonymous) => Err(PeerRefusal::Anonymous),
        (Security::Plaintext, Peer::Token) => Ok(()),
        (Security::Plaintext, _) => Err(PeerRefusal::WrongKind),
        (
            Security::MutualTls,
            Peer::Certificate {
                chain_verified: true,
            },
        ) => Ok(()),
        (Security::MutualTls, Peer::Certificate { .. }) => Err(PeerRefusal::CertificateNotVerified),
        (Security::MutualTls, Peer::Token) => Err(PeerRefusal::TokenOnEncrypted),
        (Security::MutualTls, Peer::SshKey { .. }) => Err(PeerRefusal::WrongKind),
        (
            Security::Ssh,
            Peer::SshKey {
                host_key_verified: true,
            },
        ) => Ok(()),
        (Security::Ssh, Peer::SshKey { .. }) => Err(PeerRefusal::HostKeyNotVerified),
        (Security::Ssh, Peer::Token) => Err(PeerRefusal::TokenOnEncrypted),
        (Security::Ssh, Peer::Certificate { .. }) => Err(PeerRefusal::WrongKind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_plaintext_listener_off_loopback_never_starts_whatever_else_is_set() {
        for a in [
            "0.0.0.0:7000",
            "[::]:7000",
            "192.168.1.5:7000",
            "10.0.0.1:7000",
            "8.8.8.8:7000",
            "[fe80::1]:7000",
            "[::ffff:10.0.0.1]:7000",
            "169.254.1.1:7000",
        ] {
            assert_eq!(
                check_bind(&addr(a), Security::Plaintext, true),
                Err(Refusal::PlaintextOffLoopback(addr(a))),
                "{a}"
            );
        }
    }

    #[test]
    fn plaintext_on_loopback_needs_a_token_and_every_loopback_spelling_counts() {
        for a in [
            "127.0.0.1:7000",
            "127.4.5.6:7000",
            "[::1]:7000",
            "[::ffff:127.0.0.1]:7000",
        ] {
            assert_eq!(
                check_bind(&addr(a), Security::Plaintext, true),
                Ok(()),
                "{a}"
            );
            assert_eq!(
                check_bind(&addr(a), Security::Plaintext, false),
                Err(Refusal::PlaintextWithoutToken),
                "{a}"
            );
        }
    }

    #[test]
    fn mutual_tls_and_ssh_may_listen_anywhere() {
        for a in [
            "0.0.0.0:7000",
            "[::]:7000",
            "192.168.1.5:7000",
            "127.0.0.1:7000",
        ] {
            for s in [Security::MutualTls, Security::Ssh] {
                assert_eq!(check_bind(&addr(a), s, false), Ok(()), "{a} {s:?}");
            }
        }
    }

    fn tls(client_ca_configured: bool, allow_off_loopback: bool) -> TlsBind {
        TlsBind {
            client_ca_configured,
            allow_off_loopback,
        }
    }

    #[test]
    fn a_mutual_tls_listener_needs_a_client_ca_wherever_it_listens() {
        for a in [
            "127.0.0.1:7000",
            "[::1]:7000",
            "0.0.0.0:7000",
            "10.0.0.1:7000",
        ] {
            for allow in [false, true] {
                assert_eq!(
                    check_mutual_tls_bind(&addr(a), tls(false, allow)),
                    Err(Refusal::MutualTlsWithoutClientCa),
                    "{a} allow={allow}"
                );
            }
        }
    }

    #[test]
    fn a_mutual_tls_listener_is_loopback_unless_the_operator_allowed_more() {
        for a in ["127.0.0.1:7000", "[::1]:7000", "[::ffff:127.0.0.1]:7000"] {
            assert_eq!(
                check_mutual_tls_bind(&addr(a), tls(true, false)),
                Ok(()),
                "{a}"
            );
        }
        for a in [
            "0.0.0.0:7000",
            "[::]:7000",
            "192.168.1.5:7000",
            "[fe80::1]:7000",
        ] {
            assert_eq!(
                check_mutual_tls_bind(&addr(a), tls(true, false)),
                Err(Refusal::OffLoopbackNotAllowed(addr(a))),
                "{a}"
            );
            assert_eq!(
                check_mutual_tls_bind(&addr(a), tls(true, true)),
                Ok(()),
                "{a}"
            );
        }
    }

    #[test]
    fn the_allowance_is_off_by_default_and_the_off_loopback_refusal_says_how_to_ask() {
        assert_eq!(TlsBind::default(), tls(false, false));
        let e = check_mutual_tls_bind(&addr("0.0.0.0:1"), tls(true, false))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("0.0.0.0:1") && e.contains("--allow-off-loopback"),
            "{e}"
        );
        let e = check_mutual_tls_bind(&addr("127.0.0.1:1"), tls(false, false))
            .unwrap_err()
            .to_string();
        assert!(e.contains("client") && e.contains("CA"), "{e}");
    }

    #[test]
    fn plaintext_off_loopback_stays_refused_whatever_the_tls_allowance_is() {
        // the allowance is a property of the mutual TLS listener only: check_bind knows nothing of it
        for a in ["0.0.0.0:7000", "192.168.1.5:7000"] {
            assert_eq!(
                check_bind(&addr(a), Security::Plaintext, true),
                Err(Refusal::PlaintextOffLoopback(addr(a)))
            );
        }
    }

    #[test]
    fn a_refusal_says_what_to_do_and_names_the_address() {
        let e = check_bind(&addr("0.0.0.0:1"), Security::Plaintext, true)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("0.0.0.0:1") && e.contains("mutual TLS or SSH"),
            "{e}"
        );
    }

    #[test]
    fn an_anonymous_peer_is_refused_on_every_transport() {
        for s in [Security::Plaintext, Security::MutualTls, Security::Ssh] {
            assert_eq!(check_peer(s, &Peer::Anonymous), Err(PeerRefusal::Anonymous));
        }
    }

    #[test]
    fn a_certificate_or_host_key_that_was_not_verified_is_refused() {
        assert_eq!(
            check_peer(
                Security::MutualTls,
                &Peer::Certificate {
                    chain_verified: false
                }
            ),
            Err(PeerRefusal::CertificateNotVerified)
        );
        assert_eq!(
            check_peer(
                Security::Ssh,
                &Peer::SshKey {
                    host_key_verified: false
                }
            ),
            Err(PeerRefusal::HostKeyNotVerified)
        );
        assert_eq!(
            check_peer(
                Security::MutualTls,
                &Peer::Certificate {
                    chain_verified: true
                }
            ),
            Ok(())
        );
        assert_eq!(
            check_peer(
                Security::Ssh,
                &Peer::SshKey {
                    host_key_verified: true
                }
            ),
            Ok(())
        );
    }

    #[test]
    fn a_downgrade_to_a_weaker_proof_is_refused() {
        // a token on an encrypted transport, a certificate on SSH, an SSH key on TLS, a certificate on plaintext
        assert_eq!(
            check_peer(Security::MutualTls, &Peer::Token),
            Err(PeerRefusal::TokenOnEncrypted)
        );
        assert_eq!(
            check_peer(Security::Ssh, &Peer::Token),
            Err(PeerRefusal::TokenOnEncrypted)
        );
        assert_eq!(
            check_peer(
                Security::Ssh,
                &Peer::Certificate {
                    chain_verified: true
                }
            ),
            Err(PeerRefusal::WrongKind)
        );
        assert_eq!(
            check_peer(
                Security::MutualTls,
                &Peer::SshKey {
                    host_key_verified: true
                }
            ),
            Err(PeerRefusal::WrongKind)
        );
        assert_eq!(
            check_peer(
                Security::Plaintext,
                &Peer::Certificate {
                    chain_verified: true
                }
            ),
            Err(PeerRefusal::WrongKind)
        );
    }

    #[test]
    fn only_a_token_opens_a_plaintext_loopback_listener() {
        assert_eq!(check_peer(Security::Plaintext, &Peer::Token), Ok(()));
    }
}
