use super::*;
use crate::domain::Verdict;
use crate::jev::answer::{ChoiceAnswer, ScoreAnswer};
use std::collections::BTreeMap;

pub(super) fn ans(choice: &str, probs: &[(&str, f64)], conf: f64) -> ChoiceAnswer {
    ChoiceAnswer {
        choice: choice.into(),
        probabilities: probs
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
        confidence: conf,
    }
}

pub(super) fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

pub(super) fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} vs {b}");
}

mod formulas;
mod picks;
