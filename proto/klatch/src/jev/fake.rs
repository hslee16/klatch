//! A fake transport for tests (and for the code that builds on this module): it replays a script of replies or injects
//! a status, records every request and the key it was handed, and never touches a network. [`fixtures`] holds the reply
//! shapes recorded by the live check of 2026-11-19 (#911) and the shapes the documentation gives for what was not
//! checked live, each with synthetic text only.

use super::ApiKey;
use super::error::{JevTransportError, RetryHeaders};
use super::transport::{JevRequest, JevTransport, Method};
use std::cell::RefCell;
use std::collections::VecDeque;

/// What the fake saw of one call.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub method: Method,
    pub path: &'static str,
    pub images: bool,
    pub body: Option<String>,
    pub key: String,
}

pub struct FakeTransport {
    script: RefCell<VecDeque<Result<String, JevTransportError>>>,
    /// What to answer when the script runs out; `None` means a network failure.
    after: Option<Result<String, JevTransportError>>,
    seen: RefCell<Vec<Seen>>,
}

impl FakeTransport {
    /// Answer with these replies in order, then fail as `Network`.
    pub fn replaying(script: Vec<Result<String, JevTransportError>>) -> Self {
        Self {
            script: RefCell::new(script.into()),
            after: None,
            seen: RefCell::new(vec![]),
        }
    }

    /// Answer every call with the same reply.
    pub fn saying(reply: Result<String, JevTransportError>) -> Self {
        Self {
            script: RefCell::new(VecDeque::new()),
            after: Some(reply),
            seen: RefCell::new(vec![]),
        }
    }

    /// Answer every call with an HTTP error status and this body.
    pub fn status(status: u16, body: &str) -> Self {
        Self::saying(Err(JevTransportError::status(status, body)))
    }

    pub fn calls(&self) -> usize {
        self.seen.borrow().len()
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.borrow().clone()
    }

    pub fn last_body(&self) -> Option<String> {
        self.seen.borrow().last().and_then(|s| s.body.clone())
    }
}

/// A status error with `retry-after` headers.
pub fn status_with_headers(
    status: u16,
    retry_after: Option<&str>,
    retry_after_ms: Option<&str>,
) -> JevTransportError {
    JevTransportError::Status {
        status,
        body: String::new(),
        headers: RetryHeaders {
            retry_after: retry_after.map(String::from),
            retry_after_ms: retry_after_ms.map(String::from),
        },
    }
}

impl JevTransport for FakeTransport {
    fn send(&self, key: &ApiKey, request: &JevRequest) -> Result<String, JevTransportError> {
        self.seen.borrow_mut().push(Seen {
            method: request.method,
            path: request.path,
            images: request.images,
            body: request.body.clone(),
            key: key.expose().to_string(),
        });
        match self.script.borrow_mut().pop_front() {
            Some(r) => r,
            None => self
                .after
                .clone()
                .unwrap_or(Err(JevTransportError::Network)),
        }
    }
}

/// Recorded and documented reply shapes. `LIVE` ones were seen from the real API on 2026-11-19 (#911); `DOCS` ones are
/// the shapes api__system-one.md and the primitive pages give and have not been seen live (score, box, images, the
/// 429 and 529 bodies). All text in them is synthetic.
pub mod fixtures {
    /// LIVE shape: a choice reply (probabilities summed to one, `confidence` a separate number).
    pub const LIVE_CHOICE: &str = r#"{"model":"jev-1.14.0","answers":{"verdict":{"type":"choice","choice":"allow","confidence":0.98,"probabilities":{"allow":0.99,"redact":0.0,"deny":0.01}}},"usage":{"input_tokens":120,"output_tokens":4}}"#;
    /// LIVE shape: a noul reply (`noul`, not `probability`).
    pub const LIVE_NOUL: &str = r#"{"model":"jev-1.14.0","answers":{"billing":{"type":"noul","noul":0.98}},"usage":{"input_tokens":120,"output_tokens":12}}"#;
    /// DOCS shape: a score reply.
    pub const DOCS_SCORE: &str = r#"{"model":"jev-1.14.0","answers":{"urgency":{"type":"score","score":1.7,"confidence":0.55,"legend":{"0":"Can wait","1":"Needs attention this week","2":"Needs attention today"},"probabilities":{"0":0.1,"1":0.1,"2":0.8}}},"usage":{"input_tokens":120,"output_tokens":12}}"#;
    /// DOCS shape: a box reply with a confidence.
    pub const DOCS_BOX: &str = r#"{"model":"jev-1.14.0","answers":{"mug":{"type":"box","box":{"top":0.25,"bottom":0.82,"left":0.44,"right":0.84},"confidence":0.92}},"usage":{"input_tokens":290,"output_tokens":21}}"#;
    /// DOCS shape: a box reply whose confidence is null (the model gave none).
    pub const DOCS_BOX_NULL_CONFIDENCE: &str = r#"{"model":"jev-1.14.0","answers":{"mug":{"type":"box","box":{"top":0.2,"bottom":0.66,"left":0.19,"right":0.68},"confidence":null}},"usage":{"input_tokens":285,"output_tokens":20}}"#;
    /// LIVE shape: `GET /v1/models` (it lists only the two aliases).
    pub const LIVE_MODELS: &str = r#"{"models":[{"name":"jev-latest","description":"General-purpose system one model.","release_date":"2026-09-15"},{"name":"jev-preview","description":"The most recent release, official or not.","release_date":"2026-10-14"}]}"#;
    /// LIVE: 401 for a bad or missing key (403 was also seen).
    pub const ERR_AUTH: &str =
        r#"{"detail":{"error_type":"authentication_error","message":"Invalid API key."}}"#;
    /// LIVE: 400 for an unknown model.
    pub const ERR_UNKNOWN_MODEL: &str =
        r#"{"detail":{"error_type":"api_usage_error","message":"Unknown model."}}"#;
    /// LIVE: 400 for a state longer than the context (no message at all).
    pub const ERR_CONTEXT: &str = r#"{"detail":{"error_type":"max_tokens_exceeded"}}"#;
    /// LIVE: 400 for an unknown question type.
    pub const ERR_INVALID: &str =
        r#"{"detail":{"error_type":"api_usage_error","message":"Invalid request."}}"#;
    /// UNVERIFIED shape (the 429 and 529 bodies were never triggered): a plausible body; no code may depend on it.
    pub const ERR_UNVERIFIED_BUSY: &str =
        r#"{"detail":{"error_type":"rate_limit_error","message":"Slow down."}}"#;

    /// LIVE shape: a 422 whose FastAPI style `detail` echoes the request, here a made-up `echo` text.
    pub fn echoing_422(echo: &str) -> String {
        format!(
            r#"{{"detail":[{{"type":"missing","loc":["body","model"],"msg":"Field required","input":{{"state":"{echo}"}}}}]}}"#
        )
    }
}
