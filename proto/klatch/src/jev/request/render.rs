//! Rendering of the entries and questions of a request into JSON, counting what goes in and refusing what is not allowed
//! (split from `request.rs`, #1448; no behaviour change).

use super::super::image::Image;
use super::{
    Entry, Gate, Key, Limits, MAX_CHOICES, MAX_DEPTH, MAX_LEVELS, MIN_LEVELS, NoulCriteria,
    Question, QuestionKind, REFERENCE_MARK, RequestError,
};
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;

pub(super) struct Render<'a> {
    pub(super) gate: &'a Gate,
    pub(super) limits: &'a Limits,
    pub(super) assets: Vec<Image>,
    pub(super) uses: usize,
    pub(super) use_bytes: usize,
    pub(super) chars: usize,
    pub(super) text_bytes: usize,
}

impl Render<'_> {
    fn label(&self, label: &str) -> Result<(), RequestError> {
        if self.gate.allows(label) {
            Ok(())
        } else {
            Err(RequestError::Gated)
        }
    }

    /// Count a string that will be in the body, and refuse one that could be taken for an image reference.
    fn count(&mut self, s: &str) -> Result<(), RequestError> {
        if s.contains(REFERENCE_MARK) {
            return Err(RequestError::ReferenceCollision);
        }
        self.chars = self.chars.saturating_add(s.chars().count());
        self.text_bytes = self.text_bytes.saturating_add(s.len());
        if self.chars > self.limits.max_text_chars {
            return Err(RequestError::TooManyChars);
        }
        Ok(())
    }

    fn reference(&mut self, image: &Image) -> Result<String, RequestError> {
        let at = match self.assets.iter().position(|a| a == image) {
            Some(i) => i,
            None => {
                if self.assets.len() >= self.limits.max_images {
                    return Err(RequestError::TooManyImages);
                }
                self.assets.push(image.clone());
                self.assets.len() - 1
            }
        };
        self.uses += 1;
        self.use_bytes = self.use_bytes.saturating_add(image.len());
        if self.uses > self.limits.max_image_uses {
            return Err(RequestError::TooManyImages);
        }
        Ok(format!("{REFERENCE_MARK}{at:04}-end"))
    }

    fn key(&mut self, k: &Key) -> Result<String, RequestError> {
        let name = match k {
            Key::Fixed(n) => (*n).to_string(),
            Key::Data { label, name } => {
                self.label(label)?;
                name.clone()
            }
        };
        self.count(&name)?;
        Ok(name)
    }

    /// `depth` is how many containers enclose this entry within its own root (state, instructions or criteria).
    pub(super) fn entry(&mut self, e: &Entry, depth: usize) -> Result<Value, RequestError> {
        match e {
            Entry::Sentence(parts) => self.sentence(parts, depth),
            Entry::List(items) => self.list(items, depth),
            Entry::Object(fields) => self.object(fields, depth),
            leaf => self.leaf(leaf),
        }
    }

    /// The entries that hold one value and no children.
    fn leaf(&mut self, e: &Entry) -> Result<Value, RequestError> {
        Ok(match e {
            Entry::Fixed(s) => {
                self.count(s)?;
                Value::String((*s).into())
            }
            Entry::Text { label, text } => {
                self.label(label)?;
                self.count(text)?;
                Value::String(text.clone())
            }
            Entry::Cleared(text) => {
                self.count(text)?;
                Value::String(text.clone())
            }
            Entry::Number { label, value } => {
                self.label(label)?;
                Value::Number(
                    Number::from_f64(*value)
                        .filter(|_| value.is_finite())
                        .ok_or(RequestError::NotFinite)?,
                )
            }
            Entry::Integer { label, value } => {
                self.label(label)?;
                Value::Number((*value).into())
            }
            Entry::Flag { label, value } => {
                self.label(label)?;
                Value::Bool(*value)
            }
            Entry::Image { label, image } => {
                self.label(label)?;
                Value::String(self.reference(image)?)
            }
            _ => Value::Null,
        })
    }

    fn sentence(&mut self, parts: &[Entry], depth: usize) -> Result<Value, RequestError> {
        let mut out = String::new();
        for p in parts {
            if !is_sentence_part(p) {
                return Err(RequestError::BadStructure(
                    "a sentence holds only text and images",
                ));
            }
            match self.entry(p, depth)? {
                Value::String(s) => out.push_str(&s),
                _ => return Err(RequestError::BadStructure("a sentence part is not text")),
            }
        }
        Ok(Value::String(out))
    }

    fn list(&mut self, items: &[Entry], depth: usize) -> Result<Value, RequestError> {
        if depth >= MAX_DEPTH {
            return Err(RequestError::TooDeep);
        }
        let values = items
            .iter()
            .map(|x| self.entry(x, depth + 1))
            .collect::<Result<_, _>>()?;
        Ok(Value::Array(values))
    }

    fn object(&mut self, fields: &[(Key, Entry)], depth: usize) -> Result<Value, RequestError> {
        if depth >= MAX_DEPTH {
            return Err(RequestError::TooDeep);
        }
        let mut m = Map::new();
        for (k, v) in fields {
            let name = self.key(k)?;
            let v = self.entry(v, depth + 1)?;
            if m.insert(name, v).is_some() {
                return Err(RequestError::BadStructure("an object key repeats"));
            }
        }
        Ok(Value::Object(m))
    }

    fn instructions(
        &mut self,
        i: &Option<Entry>,
        m: &mut Map<String, Value>,
    ) -> Result<(), RequestError> {
        if let Some(e) = i {
            let v = self.entry(e, 0)?;
            m.insert("instructions".into(), v);
        }
        Ok(())
    }

    pub(super) fn question(&mut self, q: &Question) -> Result<Value, RequestError> {
        self.count(q.id)?;
        let mut m = Map::new();
        match &q.kind {
            QuestionKind::Choice {
                instructions,
                criteria,
            } => self.choice(instructions, criteria, &mut m)?,
            QuestionKind::Score {
                instructions,
                criteria,
            } => self.score(instructions, criteria, &mut m)?,
            QuestionKind::Noul {
                instructions,
                criteria,
            } => self.noul(instructions, criteria, &mut m)?,
            QuestionKind::Box { instructions } => {
                m.insert("type".into(), "box".into());
                self.instructions(instructions, &mut m)?;
            }
        }
        Ok(Value::Object(m))
    }

    fn choice(
        &mut self,
        instructions: &Option<Entry>,
        criteria: &[(Key, Entry)],
        m: &mut Map<String, Value>,
    ) -> Result<(), RequestError> {
        m.insert("type".into(), "choice".into());
        self.instructions(instructions, m)?;
        if criteria.is_empty() || criteria.len() > MAX_CHOICES {
            return Err(RequestError::BadQuestion("a choice takes 1 to 255 options"));
        }
        let mut seen = BTreeSet::new();
        let mut c = Map::new();
        for (k, v) in criteria {
            let name = self.key(k)?;
            if name.trim().is_empty() {
                return Err(RequestError::BadQuestion("an option name is empty"));
            }
            if !seen.insert(name.clone()) {
                return Err(RequestError::BadQuestion("an option name repeats"));
            }
            c.insert(name, self.entry(v, 1)?);
        }
        m.insert("criteria".into(), Value::Object(c));
        Ok(())
    }

    fn score(
        &mut self,
        instructions: &Option<Entry>,
        criteria: &[Entry],
        m: &mut Map<String, Value>,
    ) -> Result<(), RequestError> {
        m.insert("type".into(), "score".into());
        self.instructions(instructions, m)?;
        if !(MIN_LEVELS..=MAX_LEVELS).contains(&criteria.len()) {
            return Err(RequestError::BadQuestion("a score takes 2 to 10 levels"));
        }
        let levels: Vec<Value> = criteria
            .iter()
            .map(|l| self.entry(l, 1))
            .collect::<Result<_, _>>()?;
        m.insert("criteria".into(), Value::Array(levels));
        Ok(())
    }

    fn noul(
        &mut self,
        instructions: &Option<Entry>,
        criteria: &Option<NoulCriteria>,
        m: &mut Map<String, Value>,
    ) -> Result<(), RequestError> {
        m.insert("type".into(), "noul".into());
        self.instructions(instructions, m)?;
        let Some(c) = criteria else { return Ok(()) };
        let mut o = Map::new();
        if let Some(t) = &c.when_true {
            o.insert("true".into(), self.entry(t, 1)?);
        }
        if let Some(f) = &c.when_false {
            o.insert("false".into(), self.entry(f, 1)?);
        }
        if !o.is_empty() {
            m.insert("criteria".into(), Value::Object(o));
        }
        Ok(())
    }
}

fn is_sentence_part(e: &Entry) -> bool {
    matches!(
        e,
        Entry::Fixed(_) | Entry::Text { .. } | Entry::Cleared(_) | Entry::Image { .. }
    )
}
