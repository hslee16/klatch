// ---------------------------------------------------------------------------------------------------------------
// is_loopback and check_bind (session/policy.rs): a plaintext listener only on loopback and only with a token.
// Bound: none on the address (all 2^32 IPv4 and all 2^128 IPv6 addresses, any port); the Security and token inputs
// are exhausted.
// Not covered: whether a transport calls check_bind at all (a caller property), name resolution.
// ---------------------------------------------------------------------------------------------------------------

use crate::session::policy::{Refusal, Security, check_bind, is_loopback};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

fn any_ip() -> IpAddr {
    if kani::any() {
        IpAddr::V4(Ipv4Addr::from(kani::any::<[u8; 4]>()))
    } else {
        IpAddr::V6(Ipv6Addr::from(kani::any::<[u16; 8]>()))
    }
}

#[kani::proof]
#[kani::solver(cadical)]
fn loopback_is_exactly_127_slash_8_and_the_ipv6_forms() {
    let ip = any_ip();
    let expected = match ip {
        IpAddr::V4(v) => v.octets()[0] == 127,
        IpAddr::V6(v) => {
            let s = v.segments();
            let v6_loopback = s == [0, 0, 0, 0, 0, 0, 0, 1];
            let mapped_127 = s[..5] == [0; 5] && s[5] == 0xffff && (s[6] >> 8) == 127;
            v6_loopback || mapped_127
        }
    };
    assert_eq!(is_loopback(&ip), expected);
    // the addresses that listen on every interface are never loopback
    assert!(!is_loopback(&IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
    assert!(!is_loopback(&IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
}

#[kani::proof]
#[kani::solver(cadical)]
fn check_bind_allows_plaintext_only_on_loopback_with_a_token() {
    let addr = SocketAddr::new(any_ip(), kani::any());
    let token: bool = kani::any();
    let security = match kani::any::<u8>() % 3 {
        0 => Security::Plaintext,
        1 => Security::MutualTls,
        _ => Security::Ssh,
    };
    let result = check_bind(&addr, security, token);
    if security == Security::Plaintext {
        assert_eq!(result.is_ok(), is_loopback(&addr.ip()) && token);
        if !is_loopback(&addr.ip()) {
            assert_eq!(result, Err(Refusal::PlaintextOffLoopback(addr)));
        }
    } else {
        assert!(result.is_ok());
    }
}
