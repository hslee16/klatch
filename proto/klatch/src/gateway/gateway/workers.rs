//! Issued credentials for workers (#121, #120, #161). A launcher asks for a credential for a member; the gateway
//! makes a token, keeps only its hash, sets the member's clearance and parent, and audits the issue. The token proves
//! the member to whoever holds it until it expires or is revoked, and a principal made from it is `Verified`, so a
//! message it posts is marked verified. That is all it is: a bearer credential with an expiry, not isolation from a
//! hostile process of the same user (DESIGN section 32 item 13).
//!
//! A failed verification is recorded under the rule `credential`, which is deliberately not one of the safety rules
//! the circuit breaker counts: anyone can send a wrong token under a worker's name, and that must not be able to
//! stop the worker.

use super::*;
use crate::keys::Token;
use crate::store::NewCredential;

/// The longest a credential may live: a worker is short-lived, and a long-lived one should be reissued.
pub const MAX_CREDENTIAL_TTL_SECS: i64 = 30 * 24 * 3600;

/// How long a credential lives from issue at most, when the launcher asks for no other cap (#161): renewal on use
/// never passes it.
pub const DEFAULT_CREDENTIAL_MAX_LIFE_SECS: i64 = 24 * 3600;

/// How many audit entries the listing looks through for each credential's last renewal.
const AUDIT_SCAN: usize = 10_000;

/// What a launcher asks for.
pub struct CredentialSpec<'a> {
    pub member: &'a str,
    /// The highest label the worker may read and post, at most the agent cap.
    pub clearance: &'a str,
    /// The idle timeout in seconds: the credential lives this long from issue and, while it is used, from each use.
    pub ttl: i64,
    /// The hard cap in seconds from issue that renewal never passes; `None` is the default (24 hours), at most 30 days.
    pub max_life: Option<i64>,
    /// The member that launched the worker, shown in the audit.
    pub parent: Option<&'a str>,
}

/// A credential as an administrator sees it: no hash, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialInfo {
    pub member: String,
    pub clearance: String,
    pub parent: String,
    pub issued: i64,
    pub expires: i64,
    /// The moment renewal never passes (0: a credential from before renewal, which is never renewed).
    pub cap: i64,
    /// When the credential was last renewed on use, from the audit; `None` if it never was (or the audit no longer
    /// reaches back that far).
    pub last_renewed: Option<i64>,
    /// `live`, `expired` or `revoked`.
    pub state: &'static str,
}

fn refused(reason: &str) -> Error {
    Error::Refused {
        rule: "credential".into(),
        reason: reason.into(),
    }
}

/// Prove a member from its token, at time `now`. The token is checked first, so a caller without it learns nothing
/// about whether the member has a credential or what state it is in. A free function over the store because the CLI
/// has to know who it is before it can build a gateway.
pub fn check_credential<S: Store>(
    store: &S,
    now: i64,
    member: &str,
    token: &Token,
) -> Result<Principal> {
    if !valid_name(member) {
        return Err(refused("the credential is not valid"));
    }
    let rec = store
        .credential(member)?
        .filter(|r| token.matches(&r.token_hash))
        .ok_or_else(|| refused("the credential is not valid"))?;
    if rec.revoked {
        return Err(refused("the credential was revoked"));
    }
    if now >= rec.expires {
        return Err(refused("the credential expired"));
    }
    Ok(Principal::verified(member))
}

/// The shape checks on what a launcher asks for: a usable name, a lifetime in range, a sensible parent.
fn check_spec(spec: &CredentialSpec) -> Result<()> {
    let CredentialSpec {
        member,
        ttl,
        parent,
        max_life,
        ..
    } = *spec;
    if !valid_name(member) || crate::domain::is_reserved_name(member) {
        return Err(Error::Invalid(format!(
            "not a usable worker name: {member}"
        )));
    }
    if !(1..=MAX_CREDENTIAL_TTL_SECS).contains(&ttl) {
        return Err(Error::Invalid(format!(
            "a credential lives from 1 second to {MAX_CREDENTIAL_TTL_SECS} seconds"
        )));
    }
    if let Some(m) = max_life
        && !(1..=MAX_CREDENTIAL_TTL_SECS).contains(&m)
    {
        return Err(Error::Invalid(format!(
            "a credential's cap is from 1 second to {MAX_CREDENTIAL_TTL_SECS} seconds"
        )));
    }
    match parent {
        Some(p) if !valid_name(p) || p == member => Err(Error::Invalid("bad parent member".into())),
        _ => Ok(()),
    }
}

/// The audit detail of an issue: clearance, parent (or `none`) and lifetime.
fn issue_detail(spec: &CredentialSpec) -> String {
    let parent = spec.parent.filter(|p| !p.is_empty()).unwrap_or("none");
    format!(
        "clearance={} parent={parent} ttl={}s max-life={}s",
        spec.clearance,
        spec.ttl,
        spec.max_life.unwrap_or(DEFAULT_CREDENTIAL_MAX_LIFE_SECS)
    )
}

impl<S: Store> Gateway<S> {
    /// Issue a credential for a worker and return its token (the only time it exists outside the worker). The
    /// worker's clearance is set to the one it carries, so every check that follows applies it.
    pub fn admin_issue_credential(&self, who: &Principal, spec: &CredentialSpec) -> Result<Token> {
        check_spec(spec)?;
        let CredentialSpec {
            member,
            clearance,
            ttl,
            max_life,
            parent,
        } = *spec;
        self.check_grantable(clearance)?;
        let token =
            Token::generate().map_err(|e| Error::Backend(format!("no random source: {e}")))?;
        let parent_text = parent.unwrap_or("");
        self.administer(who, ("issue-credential", member), |now| {
            self.store.issue_credential(
                &NewCredential {
                    member,
                    token_hash: &token.hash(),
                    clearance,
                    parent: parent_text,
                    ttl,
                    max_life: max_life.unwrap_or(DEFAULT_CREDENTIAL_MAX_LIFE_SECS),
                },
                now,
            )?;
            self.store
                .policy_set(&format!("clearance:{member}"), Some(clearance), now)?;
            if parent.is_some() {
                self.store.note_parent(member, parent_text, now)?;
            }
            Ok(issue_detail(spec))
        })?;
        Ok(token)
    }

    /// Prove a member from its token (see `check_credential`), recording a failure in the refusal record.
    pub fn verify_credential(&self, member: &str, token: &Token) -> Result<Principal> {
        check_credential(&self.store, self.now(), member, token).inspect_err(|e| {
            self.note_refusal(&Principal::unattributed(member), ("", "credential"), e);
        })
    }

    /// A worker's session ended: revoke its own credential. Only a verified principal can, and only its own, so a
    /// typed name cannot end somebody else's. Returns whether a credential was revoked.
    pub fn end_credential(&self, who: &Principal) -> Result<bool> {
        check_member(who)?;
        if who.attribution != crate::domain::Attribution::Verified {
            return Ok(false);
        }
        let now = self.now();
        if !self.store.revoke_credential(&who.member, now)? {
            return Ok(false);
        }
        self.store.record_audit(
            &crate::store::NewAudit::new("end-credential", &who.member, "session ended"),
            now,
        )?;
        Ok(true)
    }

    /// Revoke a worker's credential: its token stops working at once.
    pub fn admin_revoke_credential(&self, who: &Principal, member: &str) -> Result<()> {
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.administer(who, ("revoke-credential", member), |now| {
            if self.store.revoke_credential(member, now)? {
                Ok(String::new())
            } else {
                Err(Error::NotFound(format!("{member} has no live credential")))
            }
        })
    }

    /// Every credential and where it stands.
    pub fn admin_credentials(&self, who: &Principal) -> Result<Vec<CredentialInfo>> {
        check_member(who)?;
        self.check_guard(who, &Operation::admin("credentials", ""))?;
        let now = self.now();
        let audit = self.store.audit_log(AUDIT_SCAN)?;
        Ok(self
            .store
            .credentials()?
            .into_iter()
            .map(|c| CredentialInfo {
                // the audit is newest first; an entry from before this credential was issued is an earlier one's
                last_renewed: audit
                    .iter()
                    .find(|a| {
                        a.action == "renew-credential" && a.target == c.member && a.ts >= c.issued
                    })
                    .map(|a| a.ts),
                cap: c.cap,
                state: if c.revoked {
                    "revoked"
                } else if now >= c.expires {
                    "expired"
                } else {
                    "live"
                },
                member: c.member,
                clearance: c.clearance,
                parent: c.parent,
                issued: c.issued,
                expires: c.expires,
            })
            .collect())
    }
}
