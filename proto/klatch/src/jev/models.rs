//! `GET /v1/models` as a typed request and response (api__list-models.md, models.md). It is the cheap check of an API
//! key (a 200 means the key works) and of the pinned model. The list holds the aliases (`jev-latest`, `jev-preview`)
//! and, per models.md, versioned ids such as `jev-1.14.0` are accepted whether or not they are listed, so a pinned
//! model that is not in the list is normal ([`ModelStatus::NotListed`]), not a failure. Listing an alias never makes the
//! config accept it: [`super::JevConfig`] still refuses every name that is not a pinned version.

use super::error::JevError;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub name: String,
    pub description: String,
    /// `YYYY-MM-DD`.
    pub release_date: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelList {
    models: Vec<ModelInfo>,
}

impl ModelList {
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    pub fn contains(&self, name: &str) -> bool {
        self.models.iter().any(|m| m.name == name)
    }
}

/// Whether the pinned model is in the list. `NotListed` is not an error: versioned ids need not be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Listed,
    NotListed,
}

fn date_ok(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// Parse a listing strictly: an object with exactly `models`, an array of objects with exactly `name`, `description`
/// and `release_date` (strings; a non-empty unique name and a `YYYY-MM-DD` date).
pub fn parse_model_list(body: &str) -> Result<ModelList, JevError> {
    let bad = JevError::Malformed;
    let v: Value = serde_json::from_str(body).map_err(|_| bad("not JSON"))?;
    let o = v.as_object().ok_or(bad("the listing is not an object"))?;
    if o.len() != 1 {
        return Err(bad("the listing has an unexpected field"));
    }
    let arr = o
        .get("models")
        .and_then(Value::as_array)
        .ok_or(bad("no models array"))?;
    let mut models = vec![];
    let mut names = BTreeSet::new();
    for e in arr {
        let m = e.as_object().ok_or(bad("a model is not an object"))?;
        if m.len() != 3 {
            return Err(bad("a model has an unexpected shape"));
        }
        let text = |k: &str| {
            m.get(k)
                .and_then(Value::as_str)
                .ok_or(bad("a model field is missing"))
        };
        let (name, description, release_date) =
            (text("name")?, text("description")?, text("release_date")?);
        if name.is_empty() || !date_ok(release_date) {
            return Err(bad("a model name or date is not valid"));
        }
        if !names.insert(name.to_string()) {
            return Err(bad("a model name repeats"));
        }
        models.push(ModelInfo {
            name: name.into(),
            description: description.into(),
            release_date: release_date.into(),
        });
    }
    Ok(ModelList { models })
}

#[cfg(test)]
mod tests;
