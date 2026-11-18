//! The transport seam (DESIGN section 20 rule 11, layer (b); item 53): a `Listener` accepts peers and a `Connector`
//! reaches one, and each yields a `Connection`, a byte stream plus the `PeerEvidence` the transport gathered. The
//! session loop runs over any such stream and never learns how the bytes travel; the `Identity` decides who the
//! peer is. No transport that opens a socket lives here: the only implementation is a fake in the tests.

use crate::domain::{Principal, Result};
use crate::gateway::{Identity, PeerEvidence};
use std::io::{self, BufRead, Write};

/// One accepted or established peer: how to read it, how to write it, and what is known about it.
pub struct Connection {
    pub input: Box<dyn BufRead + Send>,
    pub output: Box<dyn Write + Send>,
    pub evidence: PeerEvidence,
    /// How to stop the streams when the session ends (shut the socket): it must make a blocked read of `input` and
    /// a blocked write of `output` return (#146, #147). `None` for a stream that cannot be stopped from outside.
    pub closer: Option<crate::session::Closer>,
}

/// The serving side of a transport: hands out one `Connection` per peer.
pub trait Listener {
    /// The next peer, or `None` when the listener has closed for good.
    fn accept(&mut self) -> io::Result<Option<Connection>>;
}

/// The calling side of a transport: opens a `Connection` to a gateway. Its evidence describes the far end.
pub trait Connector {
    fn connect(&mut self) -> io::Result<Connection>;
}

/// Maps evidence an operator registered to a member (like an `authorized_keys` file). Anything not in the table,
/// or not matching it exactly, is unattributed (R23): the peer still gets a session, but its messages are marked
/// unverified. Stronger kinds of evidence (key, certificate, bearer) yield a verified principal on a match; a
/// process found by pid and uid is same-host evidence and stays unattributed even when mapped.
#[derive(Default)]
pub struct EvidenceTable {
    entries: Vec<(PeerEvidence, String)>,
}

impl EvidenceTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `evidence` as `member`. Builder style, so a table reads as one expression.
    pub fn allow(mut self, evidence: PeerEvidence, member: &str) -> Self {
        self.entries.push((evidence, member.to_string()));
        self
    }

    fn lookup(&self, peer: &PeerEvidence) -> Option<&str> {
        self.entries
            .iter()
            .find(|(registered, member)| same_peer(registered, peer, member))
            .map(|(_, member)| member.as_str())
    }
}

/// Whether `peer` is the registered evidence. An SSH key matches by fingerprint, and a forced-command member
/// the peer carries must name the member registered, so a key moved to a different forced command is not trusted.
fn same_peer(registered: &PeerEvidence, peer: &PeerEvidence, member: &str) -> bool {
    use PeerEvidence::{Bearer, Certificate, Process, SshKey};
    match (registered, peer) {
        (
            SshKey { fingerprint: a, .. },
            SshKey {
                fingerprint: b,
                forced_member,
            },
        ) => a == b && forced_member.as_deref().is_none_or(|f| f == member),
        (Certificate { subject: a }, Certificate { subject: b }) => a == b,
        (Bearer { token: a }, Bearer { token: b }) => a == b,
        (Process { pid: p, uid: u }, Process { pid: q, uid: v }) => p == q && u == v,
        _ => false,
    }
}

/// The unattributed name given to a peer nobody registered. It names the kind of evidence and nothing from it,
/// so a token or a key never reaches a message or a log.
fn unmapped(peer: &PeerEvidence) -> Principal {
    Principal::unattributed(&format!("unmapped-{}", peer.kind()))
}

impl Identity for EvidenceTable {
    fn resolve(&self, evidence: &PeerEvidence) -> Result<Principal> {
        if let PeerEvidence::Credential { member } = evidence {
            return crate::gateway::LocalIdentity.resolve(&PeerEvidence::Credential {
                member: member.clone(),
            });
        }
        Ok(match self.lookup(evidence) {
            Some(member) if matches!(evidence, PeerEvidence::Process { .. }) => {
                Principal::unattributed(member)
            }
            Some(member) => Principal::verified(member),
            None => unmapped(evidence),
        })
    }
}
