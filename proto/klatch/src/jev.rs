//! Jev as a judge and a facilitator advisor, its own integration (#898, epic #180 and #10).
//!
//! Jev (Typesafe) is a decision model, not a text generator: one request carries a pinned `model`, a `state` and a
//! dictionary of `questions` keyed by id; the reply is `{model, answers, usage}`, `answers` keyed by question id, and
//! carries no reasoning. This module covers the whole documented API except the real HTTP transport (part B, #871),
//! so nothing here touches a network. The parts, each in its own file:
//!
//! - [`request`]: all four question kinds (choice, score, noul, box), the state as a string or structured value,
//!   structured instructions and criteria, images in state, instructions and criteria, the label gate over everything
//!   sent, and the documented request rules (32 MiB, nesting of 64, finite numbers, no BOM) plus a configured bound
//!   on text and images, all checked before anything is sent.
//! - [`image`]: PNG and JPEG checked against the documented limits, base64 without a dependency.
//! - [`answer`]: strict parsing of the reply for every kind.
//! - [`error`]: a typed error for every documented status, carrying the status and at most `error_type`, never a body.
//! - [`retry`]: the retry policy as a pure component, with the clock, the sleeper and the jitter injected.
//! - [`models`]: `GET /v1/models`, the key check and the model check.
//! - [`decision`]: confidence against probability, the noul screen, the judge's verdict and the router's `pick`.
//! - [`transport`] and [`fake`]: the seam, and a fake that replays recorded fixtures and injects statuses.
//!
//! Three rules hold throughout. (1) Everything fails closed: any unexpected shape, missing key, out-of-range number,
//! unknown choice, low confidence, refused request, or transport error ends as `Defer` (judge) or as the rules router's
//! own decision (facilitator). (2) What Jev reads is gated by label with the strictly-below-the-ceiling rule of #872,
//! for message text, structured facts, keys and images alike. (3) A Jev answer is advice to a deterministic guard: text
//! it reads can steer its verdict, so it can only pick among candidates the guard already allows, never name anything
//! else and never widen anything. The API key is read from `TYPESAFE_API_KEY` at call time and lives only in an
//! [`ApiKey`], whose Debug and Display never show it.

use crate::domain::{
    HostedTransport, LabelChain, Lean, Reading, ReadingTransport, Verdict, label_below_ceiling,
};
use std::fmt;

pub mod answer;
pub mod decision;
pub mod error;
pub mod facilitate;
pub mod fake;
#[cfg(feature = "jev-http")]
pub mod http;
pub mod image;
pub mod models;
pub mod request;
pub mod retry;
pub mod transport;

pub use answer::{
    Answer, Answers, BoxAnswer, ChoiceAnswer, SUM_TOLERANCE, ScoreAnswer, Usage, parse_response,
};
pub use decision::{
    Band, NONE_CHOICE, NoulThresholds, REDACTED_PLACEHOLDER, choice_confidence, judge_verdict,
    noul_confidence, pick, pick_screened, score_confidence, top_to_second_ratio,
};
pub use error::{ErrorType, JevError, JevTransportError, RetryHeaders, http_error};
pub use image::{Image, ImageError, ImageFormat};
pub use models::{ModelInfo, ModelList, ModelStatus, parse_model_list};
pub use request::{
    Entry, Gate, Key, Limits, NoulCriteria, Prepared, Question, QuestionKind, RequestError,
    build_request,
};
pub use retry::{Clock, Jitter, RetryPolicy, Sleeper, Timing};
pub use transport::{JevRequest, JevTransport, Method};

/// The environment variable the API key is read from, and the only place it is read from.
pub const KEY_ENV: &str = "TYPESAFE_API_KEY";

/// The id of the one question the judge asks.
pub const JUDGE_QUESTION_ID: &str = "verdict";

/// The id of the routing question.
pub const ROUTE_QUESTION_ID: &str = "route";

/// The API key. Debug and Display show nothing of it.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: String) -> Self {
        Self(key)
    }

    /// Read the key through `lookup` (the environment in production). An empty or blank value is no key.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<ApiKey> {
        lookup(KEY_ENV)
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .map(ApiKey)
    }

    /// Read `TYPESAFE_API_KEY` now.
    pub fn from_env() -> Option<ApiKey> {
        Self::from_lookup(|n| std::env::var(n).ok())
    }

    /// The secret itself, for the transport to put in its authorization header and nowhere else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The settings: a pinned model version, the confidence threshold and the bounds on what one request may carry. There
/// is no default for the model or the threshold, so Jev is off until both are chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct JevConfig {
    model: String,
    threshold: f64,
    limits: Limits,
}

impl JevConfig {
    /// `model` is a pinned version in the form the API accepts, `jev-X.Y.Z` with three numeric parts (the bare
    /// `1.14.0` is refused by the API); never `latest`, `jev-latest`, `jev-preview` or anything else. `threshold`
    /// is in (0, 1].
    pub fn new(model: &str, threshold: f64) -> Result<JevConfig, String> {
        let version = model.strip_prefix("jev-").unwrap_or("");
        let parts: Vec<&str> = version.split('.').collect();
        let numeric = |p: &&str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
        if parts.len() != 3 || !parts.iter().all(numeric) {
            return Err(
                "the Jev model must be a pinned version such as jev-1.14.0, never latest".into(),
            );
        }
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
            return Err("the Jev confidence threshold must be above 0 and at most 1".into());
        }
        Ok(JevConfig {
            model: model.to_string(),
            threshold,
            limits: Limits::default(),
        })
    }

    /// Replace the bounds on text and images (see [`Limits`]).
    pub fn with_limits(mut self, limits: Limits) -> JevConfig {
        self.limits = limits;
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}

type KeySource = Box<dyn Fn() -> Option<ApiKey>>;

/// The retry policy with the injected clock, sleeper and jitter source it runs on.
struct Retrying {
    policy: RetryPolicy,
    clock: Box<dyn Clock>,
    sleeper: Box<dyn Sleeper>,
    jitter: Box<dyn Jitter>,
}

/// Builds requests, reads the key at call time, retries a 429 or 529 as the policy says, and parses replies strictly.
pub struct JevClient<T: JevTransport> {
    transport: T,
    config: JevConfig,
    key_source: KeySource,
    gate: Gate,
    retry: Option<Retrying>,
}

impl<T: JevTransport> JevClient<T> {
    /// A client with a closed gate (no labelled data leaves) and no retries; see [`Self::with_gate`] and
    /// [`Self::with_retry`].
    pub fn new(transport: T, config: JevConfig) -> Self {
        Self {
            transport,
            config,
            key_source: Box::new(ApiKey::from_env),
            gate: Gate::closed(),
            retry: None,
        }
    }

    /// Replace where the key comes from (tests only need this; it still yields the key at each call).
    pub fn with_key_source(mut self, source: Box<dyn Fn() -> Option<String>>) -> Self {
        self.key_source = Box::new(move || source().map(ApiKey::new));
        self
    }

    /// The label gate every request is checked against.
    pub fn with_gate(mut self, gate: Gate) -> Self {
        self.gate = gate;
        self
    }

    /// Retry a 429 or 529 under `policy`, on this clock, sleeper and jitter source.
    pub fn with_retry(
        mut self,
        policy: RetryPolicy,
        clock: Box<dyn Clock>,
        sleeper: Box<dyn Sleeper>,
        jitter: Box<dyn Jitter>,
    ) -> Self {
        self.retry = Some(Retrying {
            policy,
            clock,
            sleeper,
            jitter,
        });
        self
    }

    pub fn config(&self) -> &JevConfig {
        &self.config
    }

    fn key(&self) -> Result<ApiKey, JevError> {
        (self.key_source)().ok_or(JevError::NoKey)
    }

    fn send(&self, key: &ApiKey, request: &JevRequest) -> Result<String, JevError> {
        let op = || self.transport.send(key, request);
        let result = match &self.retry {
            Some(r) => retry::run_with_retry(
                &r.policy,
                &Timing::new(&*r.clock, &*r.sleeper, &*r.jitter),
                op,
            ),
            None => op(),
        };
        result.map_err(|e| match e {
            JevTransportError::Network => JevError::Unreachable,
            JevTransportError::Status { status, body, .. } => http_error(status, &body),
        })
    }

    /// Ask the questions about `state`. The request is built and checked first (label gate, request rules, bounds);
    /// a refusal sends nothing.
    pub fn ask(&self, state: &Entry, questions: &[Question]) -> Result<Answers, JevError> {
        let prepared = build_request(
            self.config.model(),
            state,
            questions,
            &self.gate,
            self.config.limits(),
        )
        .map_err(JevError::Refused)?;
        let key = self.key()?;
        let reply = self.send(&key, &JevRequest::system_one(&prepared))?;
        parse_response(&reply, self.config.model(), questions)
    }

    /// `GET /v1/models`: the models and aliases this key can use.
    pub fn list_models(&self) -> Result<ModelList, JevError> {
        let key = self.key()?;
        parse_model_list(&self.send(&key, &JevRequest::models())?)
    }

    /// A cheap check of the API key: a listing that comes back means the key works.
    pub fn check_key(&self) -> Result<(), JevError> {
        self.list_models().map(|_| ())
    }

    /// Whether the pinned model is in the listing. Not listed is normal for a versioned id (models.md); what proves the
    /// pinned model is the `model` of every reply, which must equal it.
    pub fn check_model(&self) -> Result<ModelStatus, JevError> {
        Ok(if self.list_models()?.contains(self.config.model()) {
            ModelStatus::Listed
        } else {
            ModelStatus::NotListed
        })
    }
}

/// The judge's one question. Its wording is fixed; the message text only ever goes in `state`.
pub fn judge_question() -> Question {
    Question::choice(
        JUDGE_QUESTION_ID,
        "Should the message in the state cross to a reader of lower clearance?",
        &[
            ("allow", "send the message unchanged"),
            ("redact", "withhold the wording of the message"),
            ("deny", "block the message"),
        ],
    )
}

/// Jev as the transport of the hosted judge (#53): the label gate stays in `HostedJudge`, which calls this only for
/// text strictly below the ceiling. The reply maps `allow`, `redact`, `deny` to a verdict by [`judge_verdict`]; anything
/// else is an error, which the hosted judge turns into `Defer`.
pub struct JevJudge<T: JevTransport> {
    client: JevClient<T>,
}

impl<T: JevTransport> JevJudge<T> {
    pub fn new(client: JevClient<T>) -> Self {
        Self { client }
    }

    pub fn config(&self) -> &JevConfig {
        self.client.config()
    }
}

impl<T: JevTransport> JevJudge<T> {
    /// Ask the one judge question about `text` and return the choice answer.
    fn ask_choice(&self, text: &str) -> Result<ChoiceAnswer, String> {
        let q = judge_question();
        let state = Entry::object(vec![(Key::Fixed("message"), Entry::cleared_text(text))]);
        let answers = self
            .client
            .ask(&state, std::slice::from_ref(&q))
            .map_err(|e| e.to_string())?;
        answers
            .choice(JUDGE_QUESTION_ID)
            .cloned()
            .ok_or_else(|| "no verdict answer".to_string())
    }
}

impl<T: JevTransport> HostedTransport for JevJudge<T> {
    fn screen(&self, text: &str) -> Result<Verdict, String> {
        let a = self.ask_choice(text)?;
        Ok(judge_verdict(&a, self.client.config().threshold()))
    }
}

/// Jev as the first stage of a cascade (#936): the pick, the probability of the pick and the confidence, before any
/// threshold (the cascade policy applies its own). An unknown choice is an error, which is no reading.
impl<T: JevTransport> ReadingTransport for JevJudge<T> {
    fn read(&self, text: &str) -> Result<Reading, String> {
        let a = self.ask_choice(text)?;
        let lean = match a.choice.as_str() {
            "allow" => Lean::Allow,
            "redact" => Lean::Redact,
            "deny" => Lean::Deny,
            _ => return Err("no usable verdict choice".to_string()),
        };
        Ok(Reading {
            lean,
            probability: a.chosen_probability(),
            confidence: a.confidence,
        })
    }
}

/// A fact that may leave the machine, with the label it was observed under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labelled {
    pub label: String,
    pub value: String,
}

/// A routing question and the state it reads.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteQuestion {
    pub state: Entry,
    pub question: Question,
}

/// Build the routing question for one ask, gating every structured fact (member names, room names, claim paths) like
/// text: only facts whose label is strictly below `ceiling` are included. If the sender or the topic is gated there is
/// no question at all; a gated candidate is simply left out (that only narrows). Returns the question and the candidates
/// actually offered, which are what [`pick`] must be given. `None` means: do not call Jev. The labels travel with the
/// facts, so the client's gate checks them again when the request is built.
pub fn route_question(
    chain: &LabelChain,
    ceiling: Option<&str>,
    sender: &Labelled,
    topic: &Labelled,
    candidates: &[Labelled],
) -> Option<(RouteQuestion, Vec<String>)> {
    let ok = |f: &Labelled| label_below_ceiling(chain, ceiling, &f.label);
    if !ok(sender) || !ok(topic) {
        return None;
    }
    let mut sent: Vec<&Labelled> = candidates
        .iter()
        .filter(|c| ok(c) && c.value != NONE_CHOICE)
        .collect();
    sent.sort_by(|a, b| a.value.cmp(&b.value));
    sent.dedup_by(|a, b| a.value == b.value);
    if sent.is_empty() {
        return None;
    }
    let mut criteria: Vec<(Key, Entry)> = sent
        .iter()
        .map(|c| {
            (
                Key::data(&c.label, &c.value),
                Entry::fixed("route the ask to this candidate"),
            )
        })
        .collect();
    criteria.push((
        Key::Fixed(NONE_CHOICE),
        Entry::fixed("route the ask to no candidate"),
    ));
    let question = Question::choice_with(
        ROUTE_QUESTION_ID,
        Some(Entry::fixed(
            "Which candidate should the sender's ask be routed to, or none?",
        )),
        criteria,
    );
    let state = Entry::object(vec![
        (
            Key::Fixed("sender"),
            Entry::text(&sender.label, &sender.value),
        ),
        (Key::Fixed("topic"), Entry::text(&topic.label, &topic.value)),
    ]);
    let offered = sent.iter().map(|c| c.value.clone()).collect();
    Some((RouteQuestion { state, question }, offered))
}

#[cfg(test)]
mod tests;
