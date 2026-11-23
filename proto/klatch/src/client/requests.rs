//! The request builders of a client: a post, a claim, a reference and a handoff (split from `client.rs`, #1448).

/// A typed handoff carried by a post: its kind and an opaque payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handoff<'a> {
    pub kind: &'a str,
    pub payload: Option<&'a str>,
}

/// What `Client::post` sends: the room and the text, and optionally a label and a handoff. Build it with
/// `PostRequest::new(room, body)` and the setters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostRequest<'a> {
    pub room: &'a str,
    pub body: &'a str,
    pub label: Option<&'a str>,
    pub handoff: Option<Handoff<'a>>,
    /// The scope an ask declares (#1171); only valid on a body that starts `seeking:`, and not with a handoff.
    pub scope: Option<&'a str>,
    /// The refs an ask carries to artifacts outside klatch (#1318); only valid on a body that starts `seeking:`, and not
    /// with a handoff. Empty by default.
    pub refs: &'a [RefInput<'a>],
}

/// One ref an ask carries (#1318): an open `kind` token of `[a-z0-9_.-]{1,32}`, a `value`, and optionally a short
/// `label` and a bounded inline `snippet`. klatch never fetches, opens, runs or resolves what it points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefInput<'a> {
    pub kind: &'a str,
    pub value: &'a str,
    pub label: Option<&'a str>,
    pub snippet: Option<&'a str>,
}

impl<'a> RefInput<'a> {
    pub fn new(kind: &'a str, value: &'a str) -> Self {
        Self {
            kind,
            value,
            label: None,
            snippet: None,
        }
    }

    /// A short label for the ref.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Bounded text that gives context without fetching anything. Data, never an instruction.
    pub fn snippet(mut self, snippet: &'a str) -> Self {
        self.snippet = Some(snippet);
        self
    }
}

impl<'a> PostRequest<'a> {
    pub fn new(room: &'a str, body: &'a str) -> Self {
        Self {
            room,
            body,
            label: None,
            handoff: None,
            scope: None,
            refs: &[],
        }
    }

    /// Attach refs to an ask (#1318). The gateway refuses them on anything that is not an ask, refuses a credential,
    /// a bad kind or value and a cap breached, and never cuts anything.
    pub fn refs(mut self, refs: &'a [RefInput<'a>]) -> Self {
        self.refs = refs;
        self
    }

    /// Declare the scope an ask is about (#1171): the grammar of a member scope. `could_answer` then names a member
    /// whose own scope differs. The gateway refuses a scope on anything that is not an ask.
    pub fn scope(mut self, scope: &'a str) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Post under this label instead of the room's default.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Make the post a typed handoff of this kind, with an optional payload.
    pub fn handoff(mut self, kind: &'a str, payload: Option<&'a str>) -> Self {
        self.handoff = Some(Handoff { kind, payload });
        self
    }
}

/// What `Client::claim` sends: the room and the path, and optionally a time to live and a reason. Build it with
/// `ClaimRequest::new(room, path)` and the setters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimRequest<'a> {
    pub room: &'a str,
    pub path: &'a str,
    pub ttl: Option<i64>,
    pub reason: Option<&'a str>,
}

impl<'a> ClaimRequest<'a> {
    pub fn new(room: &'a str, path: &'a str) -> Self {
        Self {
            room,
            path,
            ttl: None,
            reason: None,
        }
    }

    /// Hold the claim for this many seconds.
    pub fn ttl(mut self, secs: i64) -> Self {
        self.ttl = Some(secs);
        self
    }

    /// Say why the path is claimed.
    pub fn reason(mut self, reason: &'a str) -> Self {
        self.reason = Some(reason);
        self
    }
}
