//! Strict parsing of a Jev reply for all four question kinds (api__system-one.md, primitives__*.md). A reply is accepted
//! only when it is an object with exactly `model` (equal to the pinned model), `answers` (exactly the asked ids, each
//! exactly the shape of its question's kind) and optionally `usage`. Every number is finite and within its range. Any
//! other shape is an error, never a guess; the error says which rule and never what the reply held.

use super::error::JevError;
use super::request::{Question, QuestionKind};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

/// How far the probabilities of an answer may be from summing to one (they are returned rounded).
pub const SUM_TOLERANCE: f64 = 0.01;

/// A parsed choice answer; every field has been checked.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceAnswer {
    pub choice: String,
    pub probabilities: BTreeMap<String, f64>,
    /// How concentrated the probabilities are (see confidence.md); not the probability the choice is right.
    pub confidence: f64,
}

impl ChoiceAnswer {
    /// The probability of the chosen answer: the number a threshold for acting on it belongs on (confidence.md).
    pub fn chosen_probability(&self) -> f64 {
        self.probabilities.get(&self.choice).copied().unwrap_or(0.0)
    }
}

/// A parsed score answer. `probabilities[i]` is the probability of level `i`; `legend[i]` is the level's description as
/// the API echoed it (it can hold image references and structure; it is untrusted and is never shown by Debug).
#[derive(Clone, PartialEq)]
pub struct ScoreAnswer {
    /// The probability-weighted average level, so it can fall between levels.
    pub score: f64,
    pub confidence: f64,
    pub probabilities: Vec<f64>,
    pub legend: Vec<Value>,
}

impl fmt::Debug for ScoreAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScoreAnswer")
            .field("score", &self.score)
            .field("confidence", &self.confidence)
            .field("probabilities", &self.probabilities)
            .field("legend", &"<withheld>")
            .finish()
    }
}

impl ScoreAnswer {
    pub fn most_likely_level(&self) -> usize {
        let mut best = 0;
        for (i, p) in self.probabilities.iter().enumerate() {
            if *p > self.probabilities[best] {
                best = i;
            }
        }
        best
    }
}

/// A located box: edges as fractions of the image width (`left`, `right`) and height (`top`, `bottom`), from the
/// top-left corner; `left < right` and `top < bottom`. `confidence` is `None` when the model gave none.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxAnswer {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Choice(ChoiceAnswer),
    Score(ScoreAnswer),
    /// The probability the statement is true (a noul answer carries no `confidence`).
    Noul(f64),
    Box(BoxAnswer),
}

/// Token counts as reported. Neither is needed for anything; an absent or odd value is `None`, never trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// A parsed reply: exactly one answer per question asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Answers {
    model: String,
    answers: BTreeMap<String, Answer>,
    usage: Option<Usage>,
}

impl Answers {
    /// The versioned model that answered (equal to the pinned model).
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }

    pub fn choice(&self, id: &str) -> Option<&ChoiceAnswer> {
        match self.answers.get(id) {
            Some(Answer::Choice(c)) => Some(c),
            _ => None,
        }
    }

    pub fn noul(&self, id: &str) -> Option<f64> {
        match self.answers.get(id) {
            Some(Answer::Noul(p)) => Some(*p),
            _ => None,
        }
    }

    pub fn score(&self, id: &str) -> Option<&ScoreAnswer> {
        match self.answers.get(id) {
            Some(Answer::Score(s)) => Some(s),
            _ => None,
        }
    }

    pub fn boxed(&self, id: &str) -> Option<&BoxAnswer> {
        match self.answers.get(id) {
            Some(Answer::Box(b)) => Some(b),
            _ => None,
        }
    }
}

type Parsed<T> = Result<T, JevError>;

fn bad(why: &'static str) -> JevError {
    JevError::Malformed(why)
}

fn unit(v: &Value) -> Option<f64> {
    v.as_f64()
        .filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
}

fn has_type(o: &Map<String, Value>, kind: &str) -> bool {
    o.get("type").and_then(Value::as_str) == Some(kind)
}

fn parse_choice(v: &Value, choices: &[String]) -> Parsed<ChoiceAnswer> {
    let o = v.as_object().ok_or(bad("an answer is not an object"))?;
    if o.len() != 4 || !has_type(o, "choice") {
        return Err(bad(
            "a choice answer is not exactly type, choice, confidence, probabilities",
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

fn parse_noul(v: &Value) -> Parsed<f64> {
    let o = v
        .as_object()
        .filter(|o| o.len() == 2 && has_type(o, "noul"))
        .ok_or(bad("a noul answer is not {type, noul}"))?;
    o.get("noul")
        .and_then(unit)
        .ok_or(bad("a probability is missing or outside 0..1"))
}

/// The keys "0" to "n-1", each present once and nothing else, as the values in order.
fn by_level(m: &Map<String, Value>, n: usize) -> Option<Vec<&Value>> {
    if m.len() != n {
        return None;
    }
    (0..n).map(|i| m.get(&i.to_string())).collect()
}

fn parse_score(v: &Value, levels: usize) -> Parsed<ScoreAnswer> {
    let o = v
        .as_object()
        .filter(|o| o.len() == 5 && has_type(o, "score"))
        .ok_or(bad(
            "a score answer is not exactly type, score, confidence, legend, probabilities",
        ))?;
    let score = o
        .get("score")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s >= 0.0 && *s <= (levels - 1) as f64)
        .ok_or(bad("the score is missing or outside the scale"))?;
    let confidence = o
        .get("confidence")
        .and_then(unit)
        .ok_or(bad("confidence is missing or outside 0..1"))?;
    let legend = o
        .get("legend")
        .and_then(Value::as_object)
        .and_then(|m| by_level(m, levels))
        .ok_or(bad("the legend does not match the levels"))?
        .into_iter()
        .cloned()
        .collect();
    let probabilities: Vec<f64> = o
        .get("probabilities")
        .and_then(Value::as_object)
        .and_then(|m| by_level(m, levels))
        .ok_or(bad("the probabilities do not match the levels"))?
        .into_iter()
        .map(unit)
        .collect::<Option<_>>()
        .ok_or(bad("a probability is missing or outside 0..1"))?;
    if (probabilities.iter().sum::<f64>() - 1.0).abs() > SUM_TOLERANCE {
        return Err(bad("probabilities do not sum to one"));
    }
    Ok(ScoreAnswer {
        score,
        confidence,
        probabilities,
        legend,
    })
}

fn parse_box(v: &Value) -> Parsed<BoxAnswer> {
    let o = v
        .as_object()
        .filter(|o| o.len() == 3 && has_type(o, "box"))
        .ok_or(bad("a box answer is not exactly type, box, confidence"))?;
    let b = o
        .get("box")
        .and_then(Value::as_object)
        .filter(|b| b.len() == 4)
        .ok_or(bad("a box is not exactly top, bottom, left, right"))?;
    let edge = |k: &str| {
        b.get(k)
            .and_then(unit)
            .ok_or(bad("a box edge is missing or outside 0..1"))
    };
    let (top, bottom, left, right) = (edge("top")?, edge("bottom")?, edge("left")?, edge("right")?);
    if top >= bottom || left >= right {
        return Err(bad("a box has an edge on the wrong side of its opposite"));
    }
    let confidence = match o.get("confidence") {
        Some(Value::Null) => None,
        Some(c) => Some(unit(c).ok_or(bad("confidence is outside 0..1"))?),
        None => return Err(bad("a box answer has no confidence")),
    };
    Ok(BoxAnswer {
        top,
        bottom,
        left,
        right,
        confidence,
    })
}

fn parse_usage(v: &Value) -> Parsed<Usage> {
    let o = v.as_object().ok_or(bad("usage is not an object"))?;
    let count = |k: &str| o.get(k).and_then(Value::as_u64);
    Ok(Usage {
        input_tokens: count("input_tokens"),
        output_tokens: count("output_tokens"),
    })
}

/// Parse a reply strictly against the pinned `model` and the questions asked.
pub fn parse_response(
    body: &str,
    model: &str,
    questions: &[Question],
) -> Result<Answers, JevError> {
    let v: Value = serde_json::from_str(body).map_err(|_| bad("not JSON"))?;
    let o = v.as_object().ok_or(bad("the reply is not an object"))?;
    if o.keys()
        .any(|k| !matches!(k.as_str(), "model" | "answers" | "usage"))
    {
        return Err(bad("the reply has an unexpected field"));
    }
    if o.get("model").and_then(Value::as_str) != Some(model) {
        return Err(bad("the reply model is not the pinned model"));
    }
    let usage = o.get("usage").map(parse_usage).transpose()?;
    let answers = o
        .get("answers")
        .and_then(Value::as_object)
        .ok_or(bad("no answers"))?;
    if answers.len() != questions.len() {
        return Err(bad("the answers do not match the questions"));
    }
    let mut out = BTreeMap::new();
    for q in questions {
        let a = answers.get(q.id).ok_or(bad("an answer is missing"))?;
        let parsed = match &q.kind {
            QuestionKind::Choice { .. } => Answer::Choice(parse_choice(a, &q.choice_names())?),
            QuestionKind::Score { criteria, .. } => Answer::Score(parse_score(a, criteria.len())?),
            QuestionKind::Noul { .. } => Answer::Noul(parse_noul(a)?),
            QuestionKind::Box { .. } => Answer::Box(parse_box(a)?),
        };
        if out.insert(q.id.to_string(), parsed).is_some() {
            return Err(bad("a question id repeats"));
        }
    }
    Ok(Answers {
        model: model.to_string(),
        answers: out,
        usage,
    })
}

#[cfg(test)]
mod tests;
