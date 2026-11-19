//! Jev as a judge and a facilitator advisor, its own integration (#898, epic #180 and #10).
//!
//! Jev (Typesafe) is a decision model, not a text generator: one request carries a pinned `model`, a `state` and a
//! list of `questions`; the reply is keyed by question id and carries no reasoning. This file holds only the pure
//! parts, so nothing here touches a network: the wire types, a strict reply parser, the adapter that plugs Jev into
//! the hosted judge's [`HostedTransport`], the confidence-threshold helper for the facilitator path, and the label
//! gate for structured facts. A real HTTP transport is #871; the [`JevTransport`] trait is where it goes.
//!
//! Three rules hold throughout. (1) Everything fails closed: any unexpected shape, missing key, out-of-range number,
//! unknown choice, low confidence, missing API key or transport error ends as `Defer` (judge) or as the rules
//! router's own decision (facilitator). (2) What Jev reads is gated by label with the strictly-below-the-ceiling rule
//! of #872, for message text and for structured facts alike. (3) A Jev answer is advice to a deterministic guard:
//! text it reads can steer its verdict, so it can only pick among candidates the guard already allows, never name
//! anything else and never widen anything. The API key is read from `TYPESAFE_API_KEY` at call time and lives only
//! in an [`ApiKey`], whose Debug and Display never show it.

use crate::domain::{HostedTransport, LabelChain, Verdict, label_below_ceiling};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt;

/// The environment variable the API key is read from, and the only place it is read from.
pub const KEY_ENV: &str = "TYPESAFE_API_KEY";

/// How far the probabilities of a choice answer may be from summing to one.
pub const SUM_TOLERANCE: f64 = 0.01;

/// The extra choice every routing question carries: send nothing anywhere.
pub const NONE_CHOICE: &str = "none";

/// The id of the one question the judge asks.
pub const JUDGE_QUESTION_ID: &str = "verdict";

/// The text a redaction carries: Jev returns no text, so it cannot write a replacement, and the original is never used.
pub const REDACTED_PLACEHOLDER: &str = "[redacted by the judge]";

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

/// The settings: a pinned model version and the confidence threshold. There is no default, so Jev is off until both
/// are chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct JevConfig {
    model: String,
    threshold: f64,
}

impl JevConfig {
    /// `model` is a pinned version, `1.14.0` or `jev-1.14.0`; never `latest` or anything that is not a three-part
    /// number. `threshold` is in (0, 1].
    pub fn new(model: &str, threshold: f64) -> Result<JevConfig, String> {
        let version = model.strip_prefix("jev-").unwrap_or(model);
        let parts: Vec<&str> = version.split('.').collect();
        let numeric = |p: &&str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
        if parts.len() != 3 || !parts.iter().all(numeric) {
            return Err(
                "the Jev model must be a pinned version such as 1.14.0, never latest".into(),
            );
        }
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
            return Err("the Jev confidence threshold must be above 0 and at most 1".into());
        }
        Ok(JevConfig {
            model: model.to_string(),
            threshold,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn threshold(&self) -> f64 {
        self.threshold
    }
}

/// One question of a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub id: String,
    pub kind: QuestionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuestionKind {
    /// Pick one of `choices`.
    Choice {
        prompt: String,
        choices: Vec<String>,
    },
    /// Is the statement true, as a probability.
    Noul { statement: String },
}

impl Question {
    pub fn choice(id: &str, prompt: &str, choices: &[&str]) -> Question {
        Question {
            id: id.into(),
            kind: QuestionKind::Choice {
                prompt: prompt.into(),
                choices: choices.iter().map(|c| c.to_string()).collect(),
            },
        }
    }

    pub fn noul(id: &str, statement: &str) -> Question {
        Question {
            id: id.into(),
            kind: QuestionKind::Noul {
                statement: statement.into(),
            },
        }
    }

    /// The judge's one question. Its wording is fixed; the message text only ever goes in `state`.
    pub fn judge() -> Question {
        Question::choice(
            JUDGE_QUESTION_ID,
            "Should the message in the state cross to a reader of lower clearance? allow sends it unchanged, \
             redact withholds the wording, deny blocks it.",
            &["allow", "redact", "deny"],
        )
    }

    fn to_json(&self) -> Value {
        match &self.kind {
            QuestionKind::Choice { prompt, choices } => {
                json!({"id": self.id, "type": "choice", "question": prompt, "choices": choices})
            }
            QuestionKind::Noul { statement } => {
                json!({"id": self.id, "type": "noul", "statement": statement})
            }
        }
    }
}

/// The request body: `{model, state, questions}`.
pub fn request_json(model: &str, state: &Value, questions: &[Question]) -> String {
    json!({
        "model": model,
        "state": state,
        "questions": questions.iter().map(Question::to_json).collect::<Vec<_>>(),
    })
    .to_string()
}

/// A parsed choice answer; every field has been checked.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceAnswer {
    pub choice: String,
    pub probabilities: BTreeMap<String, f64>,
    pub confidence: f64,
}

impl ChoiceAnswer {
    /// The probability of the chosen answer.
    pub fn chosen_probability(&self) -> f64 {
        self.probabilities.get(&self.choice).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Choice(ChoiceAnswer),
    Noul(f64),
}

/// A parsed reply: exactly one answer per question asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Answers(BTreeMap<String, Answer>);

impl Answers {
    pub fn choice(&self, id: &str) -> Option<&ChoiceAnswer> {
        match self.0.get(id) {
            Some(Answer::Choice(c)) => Some(c),
            _ => None,
        }
    }

    pub fn noul(&self, id: &str) -> Option<f64> {
        match self.0.get(id) {
            Some(Answer::Noul(p)) => Some(*p),
            _ => None,
        }
    }
}

/// Why a call produced no usable answer. No variant ever holds the key or any part of a reply body.
#[derive(Debug, Clone, PartialEq)]
pub enum JevError {
    NoKey,
    Transport(String),
    Malformed(&'static str),
}

impl fmt::Display for JevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JevError::NoKey => write!(f, "no {KEY_ENV} in the environment"),
            JevError::Transport(why) => write!(f, "the Jev transport failed: {why}"),
            JevError::Malformed(why) => write!(f, "the Jev reply was refused: {why}"),
        }
    }
}

impl std::error::Error for JevError {}

fn unit(v: &Value) -> Option<f64> {
    v.as_f64()
        .filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
}

fn parse_choice(v: &Value, choices: &[String]) -> Result<ChoiceAnswer, JevError> {
    let bad = JevError::Malformed;
    let o = v.as_object().ok_or(bad("an answer is not an object"))?;
    if o.len() != 3 {
        return Err(bad(
            "a choice answer has other fields than choice, probabilities, confidence",
        ));
    }
    let choice = o
        .get("choice")
        .and_then(Value::as_str)
        .ok_or(bad("no choice"))?;
    if !choices.iter().any(|c| c == choice) {
        return Err(bad("a choice that was not offered"));
    }
    let confidence = o
        .get("confidence")
        .and_then(unit)
        .ok_or(bad("confidence is missing or outside 0..1"))?;
    let raw = o
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or(bad("no probabilities"))?;
    if raw.len() != choices.len() {
        return Err(bad("probabilities do not match the choices"));
    }
    let mut probabilities = BTreeMap::new();
    for c in choices {
        let p = raw
            .get(c)
            .and_then(unit)
            .ok_or(bad("a probability is missing or outside 0..1"))?;
        probabilities.insert(c.clone(), p);
    }
    let sum: f64 = probabilities.values().sum();
    if (sum - 1.0).abs() > SUM_TOLERANCE {
        return Err(bad("probabilities do not sum to one"));
    }
    let max = probabilities.values().copied().fold(0.0, f64::max);
    if probabilities[choice] + 1e-9 < max {
        return Err(bad("the choice is not the most probable one"));
    }
    Ok(ChoiceAnswer {
        choice: choice.to_string(),
        probabilities,
        confidence,
    })
}

/// Parse a reply strictly against the questions asked: an object with exactly the asked ids, each answer exactly the
/// shape of its kind. Anything else is an error.
pub fn parse_response(body: &str, questions: &[Question]) -> Result<Answers, JevError> {
    let v: Value = serde_json::from_str(body).map_err(|_| JevError::Malformed("not JSON"))?;
    let o: &Map<String, Value> = v
        .as_object()
        .ok_or(JevError::Malformed("the reply is not an object"))?;
    if o.len() != questions.len() {
        return Err(JevError::Malformed(
            "the answers do not match the questions",
        ));
    }
    let mut out = BTreeMap::new();
    for q in questions {
        let a = o
            .get(&q.id)
            .ok_or(JevError::Malformed("an answer is missing"))?;
        let parsed = match &q.kind {
            QuestionKind::Choice { choices, .. } => Answer::Choice(parse_choice(a, choices)?),
            QuestionKind::Noul { .. } => {
                let ob = a
                    .as_object()
                    .filter(|ob| ob.len() == 1)
                    .ok_or(JevError::Malformed("a noul answer is not {probability}"))?;
                let p = ob
                    .get("probability")
                    .and_then(unit)
                    .ok_or(JevError::Malformed(
                        "a probability is missing or outside 0..1",
                    ))?;
                Answer::Noul(p)
            }
        };
        if out.insert(q.id.clone(), parsed).is_some() {
            return Err(JevError::Malformed("a question id repeats"));
        }
    }
    Ok(Answers(out))
}

/// How a request body reaches Jev (POST, with the key as the bearer credential). `Err` is any failure. The real
/// HTTP transport is #871.
pub trait JevTransport {
    fn post(&self, key: &ApiKey, body: &str) -> Result<String, String>;
}

type KeySource = Box<dyn Fn() -> Option<ApiKey>>;

/// Builds requests, reads the key at call time, and parses replies strictly.
pub struct JevClient<T: JevTransport> {
    transport: T,
    config: JevConfig,
    key_source: KeySource,
}

impl<T: JevTransport> JevClient<T> {
    pub fn new(transport: T, config: JevConfig) -> Self {
        Self {
            transport,
            config,
            key_source: Box::new(ApiKey::from_env),
        }
    }

    /// Replace where the key comes from (tests only need this; it still yields the key at each call).
    pub fn with_key_source(mut self, source: Box<dyn Fn() -> Option<String>>) -> Self {
        self.key_source = Box::new(move || source().map(ApiKey::new));
        self
    }

    pub fn config(&self) -> &JevConfig {
        &self.config
    }

    /// Ask the questions about `state`.
    pub fn ask(&self, state: &Value, questions: &[Question]) -> Result<Answers, JevError> {
        let key = (self.key_source)().ok_or(JevError::NoKey)?;
        let body = request_json(self.config.model(), state, questions);
        let reply = self.transport.post(&key, &body).map_err(|why| {
            let scrubbed = why.replace(key.expose(), "<redacted>");
            JevError::Transport(scrubbed.chars().take(200).collect())
        })?;
        parse_response(&reply, questions)
    }
}

/// Jev as the transport of the hosted judge (#53): the label gate stays in `HostedJudge`, which calls this only for
/// text strictly below the ceiling. The reply maps `allow`, `redact`, `deny` to a verdict only when Jev is at least
/// the threshold confident in its own pick; anything else is an error, which the hosted judge turns into `Defer`.
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

impl<T: JevTransport> HostedTransport for JevJudge<T> {
    fn screen(&self, text: &str) -> Result<Verdict, String> {
        let q = Question::judge();
        let answers = self
            .client
            .ask(&json!({"message": text}), std::slice::from_ref(&q))
            .map_err(|e| e.to_string())?;
        let a = answers
            .choice(JUDGE_QUESTION_ID)
            .ok_or("no verdict answer")?;
        let t = self.client.config().threshold();
        if a.confidence < t || a.chosen_probability() < t {
            return Ok(Verdict::Defer);
        }
        Ok(match a.choice.as_str() {
            "allow" => Verdict::Allow,
            "redact" => Verdict::Redact(REDACTED_PLACEHOLDER.to_string()),
            "deny" => Verdict::Deny("the Jev judge denied it".to_string()),
            _ => Verdict::Defer,
        })
    }
}

/// The facilitator and router decision. `candidates` are the names (rooms or members) the rules and the guard already
/// allow; `rules` is what the rules router decided (`None` for nothing). Jev's answer is taken only when both its
/// confidence and its own probability reach `threshold`, and then only if it is [`NONE_CHOICE`] (narrow to nothing) or
/// one of the candidates; a name outside the candidates is ignored. In every other case the rules decision stands.
pub fn pick(
    candidates: &[String],
    rules: Option<&str>,
    answer: &ChoiceAnswer,
    threshold: f64,
) -> Option<String> {
    let rules = rules.map(String::from);
    if answer.confidence < threshold || answer.chosen_probability() < threshold {
        return rules;
    }
    if answer.choice == NONE_CHOICE {
        return None;
    }
    if candidates.contains(&answer.choice) {
        return Some(answer.choice.clone());
    }
    rules
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
    pub state: Value,
    pub question: Question,
}

pub const ROUTE_QUESTION_ID: &str = "route";

/// Build the routing question for one ask, gating every structured fact (member names, room names, claim paths) like
/// text: only facts whose label is strictly below `ceiling` are included. If the sender or the topic is gated there
/// is no question at all; a gated candidate is simply left out (that only narrows). Returns the question and the
/// candidates actually offered, which are what [`pick`] must be given. `None` means: do not call Jev.
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
    let mut sent: Vec<String> = candidates
        .iter()
        .filter(|c| ok(c) && c.value != NONE_CHOICE)
        .map(|c| c.value.clone())
        .collect();
    sent.sort();
    sent.dedup();
    if sent.is_empty() {
        return None;
    }
    let mut choices: Vec<&str> = sent.iter().map(String::as_str).collect();
    choices.push(NONE_CHOICE);
    let question = Question::choice(
        ROUTE_QUESTION_ID,
        "Which candidate should the sender's ask be routed to, or none?",
        &choices,
    );
    let state = json!({"sender": sender.value, "topic": topic.value});
    Some((RouteQuestion { state, question }, sent))
}

#[cfg(test)]
mod tests;
