//! The written-out specifications the model tests check the gateway against: the claim expiry after a park, the
//! role table and the attempts it covers. Kept apart from the state machines so each file stays short.

/// What a member holds. The real type is private to the gateway; this is the spec's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Held4 {
    Nobody,
    Librarian,
    Admin,
    Reviewer,
}
pub(super) const ROLES: [Held4; 4] = [
    Held4::Nobody,
    Held4::Librarian,
    Held4::Admin,
    Held4::Reviewer,
];

/// Every way a member can try something, one per `Operation` variant the gateway builds (an admin act once per
/// action name, since the action is what the allowlist decides on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Try {
    Join,
    Post,
    ReadAdvance,
    ReadPeek,
    Who,
    Claim,
    Release,
    Claims,
    Tail,
    Head,
    History,
    Status,
    Ack,
    Mute,
    ReviewNext,
    ReviewVerdict,
    Admin(&'static str),
}

/// The admin actions the gateway builds today plus ones it does not (a new act must be added to the spec to be
/// allowed: the allowlist is closed by default, so an unknown name is refused).
pub(super) const ADMIN_ACTIONS: [&str; 33] = [
    "close",
    "reopen",
    "remove",
    "allow",
    "clear",
    "label",
    "prune",
    "run-create",
    "run-end",
    "reap-anchors",
    "anchor-done",
    "audit",
    "refusals",
    "hook_runs",
    "cap",
    "chain",
    "scrub-add",
    "scrub-remove",
    "seal-label",
    "canary",
    "librarian",
    "admin-agent",
    "reviewer",
    "issue-credential",
    "revoke-credential",
    "paused",
    "closed",
    "credentials",
    "runs",
    "review-queue",
    "stop",
    "an-act-nobody-has-written-yet",
    "",
];

pub(super) fn every_try() -> Vec<Try> {
    let mut all = vec![
        Try::Join,
        Try::Post,
        Try::ReadAdvance,
        Try::ReadPeek,
        Try::Who,
        Try::Claim,
        Try::Release,
        Try::Claims,
        Try::Tail,
        Try::Head,
        Try::History,
        Try::Status,
        Try::Ack,
        Try::Mute,
        Try::ReviewNext,
        Try::ReviewVerdict,
    ];
    all.extend(ADMIN_ACTIONS.iter().map(|a| Try::Admin(a)));
    all
}

/// The expiry of a claim after its owner parks: renewal first; short of the return time: stretched to it; the stretch
/// of an earlier park (beyond the new return time or the claim's own ttl from now, within the earlier return time, `held`):
/// cut back to that.
pub(super) fn parked_expiry(expires: i64, ttl: i64, (now, back_by, held): (i64, i64, i64)) -> i64 {
    let renewed = expires.max(now + ttl);
    let floor = back_by.max(now + ttl);
    let earlier_stretch = held > now && (floor + 1..=held).contains(&renewed);
    if renewed < back_by {
        back_by
    } else if earlier_stretch {
        floor
    } else {
        renewed
    }
}

/// The spec table, written out and not derived from the gateway's own lists. `Some(rule)`: the role layer must
/// refuse under that rule; `None`: the role layer lets it through (another layer may still say no).
pub(super) fn role_spec(role: Held4, attempt: &Try) -> Option<&'static str> {
    let reviewing = matches!(attempt, Try::ReviewNext | Try::ReviewVerdict);
    match (role == Held4::Reviewer, reviewing) {
        (true, false) | (false, true) => return Some("reviewer"),
        (true, true) => return None,
        (false, false) => {}
    }
    if let Try::Admin(action) = attempt {
        return admin_act_spec(role, action);
    }
    let looking = matches!(
        attempt,
        Try::ReadPeek | Try::Who | Try::Claims | Try::Tail | Try::Head | Try::History
    );
    (role == Held4::Librarian && !looking).then_some("read-only")
}

/// The spec for an admin act under a role (the reviewer is handled before this).
fn admin_act_spec(role: Held4, action: &str) -> Option<&'static str> {
    const FLOORS: [&str; 4] = ["cap", "chain", "scrub-add", "scrub-remove"];
    // what an admin agent may do, and nothing else
    const ADMIN_LIST: [&str; 14] = [
        "close",
        "reopen",
        "remove",
        "allow",
        "clear",
        "label",
        "prune",
        "run-create",
        "run-end",
        "reap-anchors",
        "anchor-done",
        "audit",
        "refusals",
        "hook_runs",
    ];
    const LOOKING: [&str; 3] = ["audit", "refusals", "review-queue"];
    // the gateway is the members' one: a floor act is a floor act for everyone
    if FLOORS.contains(&action) {
        return Some("floors");
    }
    match role {
        Held4::Nobody if !matches!(action, "prune" | "hook_runs") => Some("admin-scope"),
        Held4::Admin if !ADMIN_LIST.contains(&action) => Some("admin-scope"),
        Held4::Librarian if !LOOKING.contains(&action) => Some("read-only"),
        _ => None,
    }
}
