//! Building a Jev request, strictly and locally (api__errors.md, images.md, primitives__advanced.md, models.md).
//!
//! Everything that could leave the machine is an [`Entry`], and an entry is either klatch's own fixed wording or data
//! that carries the label it was observed under. [`build_request`] walks the whole request and refuses it (fails
//! closed, nothing is sent) when any labelled leaf, key or image is not strictly below the gate's ceiling, when it
//! breaks a documented request rule (nesting at most 64 levels in each of state, instructions and criteria, finite
//! numbers only, UTF-8 JSON with no byte order mark, body at most 32 MiB), or when it is larger than the configured
//! bound on text and images. A refusal says which rule, never what the data was.

use super::image::Image;
use super::transport::{FilePart, MULTIPART_OVERHEAD};
use crate::domain::{LabelChain, label_below_ceiling};
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;
use std::fmt;

/// The documented body limit, and the limit on the request once every image reference is replaced by its image.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
/// The documented nesting limit, in each of `state`, `instructions` and `criteria`.
pub const MAX_DEPTH: usize = 64;
/// A choice takes at most this many options (the API schema).
pub const MAX_CHOICES: usize = 255;
/// A score takes between two (a scale of one says nothing) and ten levels (the API accepts one to ten).
pub const MIN_LEVELS: usize = 2;
pub const MAX_LEVELS: usize = 10;
/// Default bound on the text sent (characters of every string and key in the request). The live check of 2026-11-19
/// saw 100,000 characters accepted (18,799 tokens) and 500,000 refused with 400 `max_tokens_exceeded`; the model's
/// context is 64k tokens per request, so this default is well below it even for text that costs a token a character.
pub const DEFAULT_MAX_TEXT_CHARS: usize = 40_000;
/// The most a configuration may raise the bound to (still not above the 100,000 characters seen accepted).
pub const HARD_MAX_TEXT_CHARS: usize = 100_000;
pub const DEFAULT_MAX_IMAGES: usize = 4;
pub const DEFAULT_MAX_IMAGE_USES: usize = 8;

/// The text every image reference starts with; no text, key or id may contain it, so a reference cannot match by chance.
const REFERENCE_MARK: &str = "klatch-image-";

/// The label gate: a labelled thing is sent only when its label and the ceiling are both in the chain and the label is
/// strictly below the ceiling (the one rule of #872, [`label_below_ceiling`]). The default is closed: no ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    chain: LabelChain,
    ceiling: Option<String>,
}

impl Gate {
    pub fn new(chain: LabelChain, ceiling: Option<&str>) -> Gate {
        Gate {
            chain,
            ceiling: ceiling.map(String::from),
        }
    }

    /// A gate that lets no labelled data through (fixed wording and text a caller already gated still pass).
    pub fn closed() -> Gate {
        Gate::new(LabelChain::default(), None)
    }

    pub fn allows(&self, label: &str) -> bool {
        label_below_ceiling(&self.chain, self.ceiling.as_deref(), label)
    }
}

impl Default for Gate {
    fn default() -> Self {
        Gate::closed()
    }
}

/// An object key: klatch's own fixed name, or a name that is data (a member, a room, a claim path) with its label.
#[derive(Clone, PartialEq, Eq)]
pub enum Key {
    Fixed(&'static str),
    Data { label: String, name: String },
}

impl Key {
    pub fn data(label: &str, name: &str) -> Key {
        Key::Data {
            label: label.into(),
            name: name.into(),
        }
    }
}

/// One value of a request: the state, an instruction, a criterion. Debug never shows what it holds.
#[derive(Clone, PartialEq)]
pub enum Entry {
    Null,
    /// klatch's own wording; never gated.
    Fixed(&'static str),
    Text {
        label: String,
        text: String,
    },
    Number {
        label: String,
        value: f64,
    },
    Integer {
        label: String,
        value: i64,
    },
    Flag {
        label: String,
        value: bool,
    },
    Image {
        label: String,
        image: Image,
    },
    /// Text a caller already put through the label gate (the hosted judge does, before it calls the transport).
    Cleared(String),
    List(Vec<Entry>),
    Object(Vec<(Key, Entry)>),
    /// Text with images inside it (a reference in the middle of a sentence); parts are fixed, text, cleared or image.
    Sentence(Vec<Entry>),
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Entry(<withheld>)")
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(<withheld>)")
    }
}

impl Entry {
    pub fn fixed(s: &'static str) -> Entry {
        Entry::Fixed(s)
    }
    pub fn text(label: &str, text: &str) -> Entry {
        Entry::Text {
            label: label.into(),
            text: text.into(),
        }
    }
    pub fn number(label: &str, value: f64) -> Entry {
        Entry::Number {
            label: label.into(),
            value,
        }
    }
    pub fn int(label: &str, value: i64) -> Entry {
        Entry::Integer {
            label: label.into(),
            value,
        }
    }
    pub fn flag(label: &str, value: bool) -> Entry {
        Entry::Flag {
            label: label.into(),
            value,
        }
    }
    pub fn image(label: &str, image: Image) -> Entry {
        Entry::Image {
            label: label.into(),
            image,
        }
    }
    pub fn list(items: Vec<Entry>) -> Entry {
        Entry::List(items)
    }
    pub fn object(fields: Vec<(Key, Entry)>) -> Entry {
        Entry::Object(fields)
    }
    pub fn sentence(parts: Vec<Entry>) -> Entry {
        Entry::Sentence(parts)
    }

    /// Text the caller has already checked with the label gate (the hosted judge only).
    pub(crate) fn cleared_text(text: &str) -> Entry {
        Entry::Cleared(text.into())
    }

    /// A JSON value as an entry, every leaf and every key under one label. Refused when it nests deeper than 64 levels.
    pub fn from_value(label: &str, v: &Value) -> Result<Entry, RequestError> {
        fn go(label: &str, v: &Value, depth: usize) -> Result<Entry, RequestError> {
            Ok(match v {
                Value::Null => Entry::Null,
                Value::Bool(b) => Entry::flag(label, *b),
                Value::Number(n) => match n.as_i64() {
                    Some(i) => Entry::int(label, i),
                    None if n.is_u64() => {
                        return Err(RequestError::BadStructure(
                            "an integer too large for 64 bits is sent as a string",
                        ));
                    }
                    None => Entry::number(label, n.as_f64().ok_or(RequestError::NotFinite)?),
                },
                Value::String(s) => Entry::text(label, s),
                Value::Array(a) => {
                    if depth >= MAX_DEPTH {
                        return Err(RequestError::TooDeep);
                    }
                    Entry::List(
                        a.iter()
                            .map(|x| go(label, x, depth + 1))
                            .collect::<Result<_, _>>()?,
                    )
                }
                Value::Object(o) => {
                    if depth >= MAX_DEPTH {
                        return Err(RequestError::TooDeep);
                    }
                    Entry::Object(
                        o.iter()
                            .map(|(k, x)| Ok((Key::data(label, k), go(label, x, depth + 1)?)))
                            .collect::<Result<_, RequestError>>()?,
                    )
                }
            })
        }
        go(label, v, 0)
    }
}

/// Noul criteria: what counts as true and as false (each optional, each structured).
#[derive(Debug, Clone, PartialEq)]
pub struct NoulCriteria {
    pub when_true: Option<Entry>,
    pub when_false: Option<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuestionKind {
    /// Pick one option; `criteria` is (option name, description) in the order given.
    Choice {
        instructions: Option<Entry>,
        criteria: Vec<(Key, Entry)>,
    },
    /// Rate on an ordered scale; the position of a level is its score, from zero.
    Score {
        instructions: Option<Entry>,
        criteria: Vec<Entry>,
    },
    /// Is the statement true, as a probability.
    Noul {
        instructions: Option<Entry>,
        criteria: Option<NoulCriteria>,
    },
    /// Where is it, in an image: four edges as fractions of the width and height.
    Box { instructions: Option<Entry> },
}

/// One question of a request. The id is klatch's own (a `&'static str`), never data.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub id: &'static str,
    pub kind: QuestionKind,
}

impl Question {
    pub fn choice(
        id: &'static str,
        instructions: &'static str,
        criteria: &[(&'static str, &'static str)],
    ) -> Question {
        Question::choice_with(
            id,
            Some(Entry::fixed(instructions)),
            criteria
                .iter()
                .map(|(o, d)| (Key::Fixed(o), Entry::fixed(d)))
                .collect(),
        )
    }

    pub fn choice_with(
        id: &'static str,
        instructions: Option<Entry>,
        criteria: Vec<(Key, Entry)>,
    ) -> Question {
        Question {
            id,
            kind: QuestionKind::Choice {
                instructions,
                criteria,
            },
        }
    }

    pub fn noul(id: &'static str, instructions: &'static str) -> Question {
        Question::noul_with(id, Some(Entry::fixed(instructions)), None)
    }

    pub fn noul_with(
        id: &'static str,
        instructions: Option<Entry>,
        criteria: Option<NoulCriteria>,
    ) -> Question {
        Question {
            id,
            kind: QuestionKind::Noul {
                instructions,
                criteria,
            },
        }
    }

    pub fn score(
        id: &'static str,
        instructions: &'static str,
        levels: &[&'static str],
    ) -> Question {
        Question::score_with(
            id,
            Some(Entry::fixed(instructions)),
            levels.iter().map(|l| Entry::fixed(l)).collect(),
        )
    }

    pub fn score_with(
        id: &'static str,
        instructions: Option<Entry>,
        criteria: Vec<Entry>,
    ) -> Question {
        Question {
            id,
            kind: QuestionKind::Score {
                instructions,
                criteria,
            },
        }
    }

    pub fn locate(id: &'static str, instructions: &'static str) -> Question {
        Question::locate_with(id, Some(Entry::fixed(instructions)))
    }

    pub fn locate_with(id: &'static str, instructions: Option<Entry>) -> Question {
        Question {
            id,
            kind: QuestionKind::Box { instructions },
        }
    }

    /// The option names of a choice question, in order (empty for the other kinds).
    pub fn choice_names(&self) -> Vec<String> {
        match &self.kind {
            QuestionKind::Choice { criteria, .. } => criteria
                .iter()
                .map(|(k, _)| match k {
                    Key::Fixed(n) => (*n).to_string(),
                    Key::Data { name, .. } => name.clone(),
                })
                .collect(),
            _ => vec![],
        }
    }
}

/// The configured bounds on what one request may carry. The documented 32 MiB is a constant; these are lower, chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    max_text_chars: usize,
    max_images: usize,
    max_image_uses: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_text_chars: DEFAULT_MAX_TEXT_CHARS,
            max_images: DEFAULT_MAX_IMAGES,
            max_image_uses: DEFAULT_MAX_IMAGE_USES,
        }
    }
}

impl Limits {
    /// `max_text_chars` is 1 to [`HARD_MAX_TEXT_CHARS`]; `max_images` distinct images (0 means none), each used at most
    /// `max_image_uses` times in all (each use sends the image again, and none can be fewer than the images).
    pub fn new(
        max_text_chars: usize,
        max_images: usize,
        max_image_uses: usize,
    ) -> Result<Limits, String> {
        if max_text_chars == 0 || max_text_chars > HARD_MAX_TEXT_CHARS {
            return Err(format!(
                "the Jev text bound must be between 1 and {HARD_MAX_TEXT_CHARS} characters"
            ));
        }
        if max_image_uses < max_images {
            return Err("the Jev image use bound cannot be below the image bound".into());
        }
        Ok(Limits {
            max_text_chars,
            max_images,
            max_image_uses,
        })
    }

    pub fn max_text_chars(&self) -> usize {
        self.max_text_chars
    }
    pub fn max_images(&self) -> usize {
        self.max_images
    }
    pub fn max_image_uses(&self) -> usize {
        self.max_image_uses
    }
}

/// Why a request was refused before sending. No variant holds any data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    /// A labelled thing is not strictly below the ceiling (or its label is unknown): nothing was sent.
    Gated,
    BadModel,
    NoQuestions,
    DuplicateQuestion,
    BadQuestion(&'static str),
    BadStructure(&'static str),
    TooDeep,
    NotFinite,
    /// A text, key or id contains the text image references start with.
    ReferenceCollision,
    TooManyChars,
    TooManyImages,
    /// The body (or the request with its images expanded) is over 32 MiB.
    TooLarge,
    BoxWithoutImage,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Gated => {
                f.write_str("a part of the request is not below the label ceiling")
            }
            RequestError::BadModel => f.write_str("no model is named"),
            RequestError::NoQuestions => f.write_str("a request needs at least one question"),
            RequestError::DuplicateQuestion => f.write_str("a question id repeats"),
            RequestError::BadQuestion(why) => write!(f, "a question is not valid: {why}"),
            RequestError::BadStructure(why) => {
                write!(f, "the request structure is not valid: {why}")
            }
            RequestError::TooDeep => {
                f.write_str("a part of the request nests deeper than 64 levels")
            }
            RequestError::NotFinite => f.write_str("a number is not finite"),
            RequestError::ReferenceCollision => f.write_str("a text looks like an image reference"),
            RequestError::TooManyChars => {
                f.write_str("the request holds more text than the configured bound")
            }
            RequestError::TooManyImages => {
                f.write_str("the request holds more images than the configured bound")
            }
            RequestError::TooLarge => f.write_str("the request is over the 32 MiB limit"),
            RequestError::BoxWithoutImage => {
                f.write_str("a box question needs an image in the request")
            }
        }
    }
}

impl std::error::Error for RequestError {}

/// A request that passed every local check: the body to send and whether it needs the images header.
#[derive(Clone, PartialEq, Eq)]
pub struct Prepared {
    body: String,
    images: bool,
    /// The JSON whose assets name file parts (`part`), and the files, when the request holds images.
    multipart: Option<(String, Vec<FilePart>)>,
}

impl fmt::Debug for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Prepared({} bytes, images: {})",
            self.body.len(),
            self.images
        )
    }
}

impl Prepared {
    /// The all-JSON body (images as `data_base64`). Not what the real transport sends for an image request (it sends
    /// [`Prepared::multipart`]), and for an image request it is not checked against the 32 MiB limit, which is checked
    /// on what is sent.
    pub fn body(&self) -> &str {
        &self.body
    }
    /// The multipart form of an image request: the JSON for the `request` part (each asset names its file part) and
    /// the files, in order. `None` when the request holds no image.
    pub fn multipart(&self) -> Option<(&str, &[FilePart])> {
        self.multipart
            .as_ref()
            .map(|(j, f)| (j.as_str(), f.as_slice()))
    }
    /// True when the request holds an image or a box question, so `TypeSafe-Danger-Zone-Images: true` must be sent.
    pub fn images(&self) -> bool {
        self.images
    }
}

/// The size of a multipart body of a JSON part and files of these sizes (an upper bound).
pub fn multipart_len(json_len: usize, file_lens: &[usize]) -> usize {
    file_lens
        .iter()
        .fold(json_len, |a, n| a.saturating_add(*n))
        .saturating_add(MULTIPART_OVERHEAD.saturating_mul(file_lens.len() + 1))
}

/// The size rules: the body at most 32 MiB, and the text plus every use of every image (each use adds the image again,
/// and processing re-encodes it and can enlarge it, so an image counts twice) at most 32 MiB when expanded.
pub fn check_sizes(
    text_bytes: usize,
    image_use_bytes: usize,
    body_len: usize,
) -> Result<(), RequestError> {
    let expanded = text_bytes.saturating_add(image_use_bytes.saturating_mul(2));
    if body_len > MAX_BODY_BYTES || expanded > MAX_BODY_BYTES {
        return Err(RequestError::TooLarge);
    }
    Ok(())
}

struct Render<'a> {
    gate: &'a Gate,
    limits: &'a Limits,
    assets: Vec<Image>,
    uses: usize,
    use_bytes: usize,
    chars: usize,
    text_bytes: usize,
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
    fn entry(&mut self, e: &Entry, depth: usize) -> Result<Value, RequestError> {
        Ok(match e {
            Entry::Null => Value::Null,
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
            Entry::Sentence(parts) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        Entry::Fixed(_)
                        | Entry::Text { .. }
                        | Entry::Cleared(_)
                        | Entry::Image { .. } => match self.entry(p, depth)? {
                            Value::String(s) => out.push_str(&s),
                            _ => {
                                return Err(RequestError::BadStructure(
                                    "a sentence part is not text",
                                ));
                            }
                        },
                        _ => {
                            return Err(RequestError::BadStructure(
                                "a sentence holds only text and images",
                            ));
                        }
                    }
                }
                Value::String(out)
            }
            Entry::List(items) => {
                if depth >= MAX_DEPTH {
                    return Err(RequestError::TooDeep);
                }
                Value::Array(
                    items
                        .iter()
                        .map(|x| self.entry(x, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            Entry::Object(fields) => {
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
                Value::Object(m)
            }
        })
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

    fn question(&mut self, q: &Question) -> Result<Value, RequestError> {
        self.count(q.id)?;
        let mut m = Map::new();
        match &q.kind {
            QuestionKind::Choice {
                instructions,
                criteria,
            } => {
                m.insert("type".into(), "choice".into());
                self.instructions(instructions, &mut m)?;
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
            }
            QuestionKind::Score {
                instructions,
                criteria,
            } => {
                m.insert("type".into(), "score".into());
                self.instructions(instructions, &mut m)?;
                if !(MIN_LEVELS..=MAX_LEVELS).contains(&criteria.len()) {
                    return Err(RequestError::BadQuestion("a score takes 2 to 10 levels"));
                }
                let levels: Vec<Value> = criteria
                    .iter()
                    .map(|l| self.entry(l, 1))
                    .collect::<Result<_, _>>()?;
                m.insert("criteria".into(), Value::Array(levels));
            }
            QuestionKind::Noul {
                instructions,
                criteria,
            } => {
                m.insert("type".into(), "noul".into());
                self.instructions(instructions, &mut m)?;
                if let Some(c) = criteria {
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
                }
            }
            QuestionKind::Box { instructions } => {
                m.insert("type".into(), "box".into());
                self.instructions(instructions, &mut m)?;
            }
        }
        Ok(Value::Object(m))
    }
}

/// Build the body of a `POST /v1/systemone` request, or refuse it (see the module text). Nothing is sent by this.
pub fn build_request(
    model: &str,
    state: &Entry,
    questions: &[Question],
    gate: &Gate,
    limits: &Limits,
) -> Result<Prepared, RequestError> {
    if model.trim().is_empty() {
        return Err(RequestError::BadModel);
    }
    if questions.is_empty() {
        return Err(RequestError::NoQuestions);
    }
    let mut ids = BTreeSet::new();
    for q in questions {
        if q.id.trim().is_empty() {
            return Err(RequestError::BadQuestion("a question id is empty"));
        }
        if !ids.insert(q.id) {
            return Err(RequestError::DuplicateQuestion);
        }
    }
    let mut r = Render {
        gate,
        limits,
        assets: vec![],
        uses: 0,
        use_bytes: 0,
        chars: 0,
        text_bytes: 0,
    };
    let state_v = r.entry(state, 0)?;
    let mut qs = Map::new();
    for q in questions {
        qs.insert(q.id.to_string(), r.question(q)?);
    }
    let has_box = questions
        .iter()
        .any(|q| matches!(q.kind, QuestionKind::Box { .. }));
    if has_box && r.assets.is_empty() {
        return Err(RequestError::BoxWithoutImage);
    }
    let mut root = Map::new();
    root.insert("model".into(), model.into());
    root.insert("state".into(), state_v);
    root.insert("questions".into(), Value::Object(qs));
    let mut multipart = None;
    if !r.assets.is_empty() {
        let mut a = Map::new();
        let mut pa = Map::new();
        let mut files = vec![];
        for (i, img) in r.assets.iter().enumerate() {
            let reference = format!("{REFERENCE_MARK}{i:04}-end");
            let mut o = Map::new();
            o.insert("type".into(), "image".into());
            o.insert("format".into(), img.format().name().into());
            let mut p = o.clone();
            o.insert("data_base64".into(), img.to_base64().into());
            a.insert(reference.clone(), Value::Object(o));
            let part = format!("image-{i:04}");
            p.insert("part".into(), part.clone().into());
            pa.insert(reference, Value::Object(p));
            files.push(FilePart::new(
                part,
                img.content_type(),
                img.bytes().to_vec(),
            ));
        }
        let mut proot = root.clone();
        root.insert("assets".into(), Value::Object(a));
        proot.insert("assets".into(), Value::Object(pa));
        multipart = Some((Value::Object(proot).to_string(), files));
    }
    let body = Value::Object(root).to_string();
    debug_assert!(!body.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]));
    // what is sent is the multipart form when there are images, so that is what the limit applies to
    let sent_len = match &multipart {
        Some((json, files)) => multipart_len(
            json.len(),
            &files.iter().map(|f| f.bytes().len()).collect::<Vec<_>>(),
        ),
        None => body.len(),
    };
    check_sizes(r.text_bytes, r.use_bytes, sent_len)?;
    Ok(Prepared {
        body,
        images: !r.assets.is_empty(),
        multipart,
    })
}

#[cfg(test)]
mod tests;
