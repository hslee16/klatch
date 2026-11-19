//! The judge-eval corpus (#914): one JSON object per line in `judge-eval/corpus.jsonl`, each a candidate crossing
//! with its ground-truth verdict. The labelling rules are in `judge-eval/README.md`.
//!
//! No credential-shaped string is stored in the file. A secret is written as a placeholder such as
//! `{{fake:github}}` and expanded here, at load time, into a pseudo-random string of the right shape derived from the
//! item id, so the judge sees something realistic, the same text every run, and nothing in git can be a working
//! credential or trip push protection. [`looks_like_real_credential`] is the conservative check the validity test
//! runs over every stored text.

use klatch::domain::{LabelChain, credential_kind};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Every category an item may carry. `injection-*` items are called out on their own in the report.
pub const CATEGORIES: [&str; 16] = [
    "everyday-chat",
    "public-chatter",
    "secret-credential",
    "secret-lookalike",
    "personal-data",
    "harmless-name",
    "business-private",
    "code-log-secret",
    "code-log-clean",
    "injection-overt",
    "injection-hidden",
    "injection-lookalike",
    "scrub-term",
    "multilingual",
    "short",
    "long",
];

/// The room labels an item may be posted under or cross to (the default chain without `secret`, which no agent
/// reads).
pub const LABELS: [&str; 3] = ["public", "internal", "private"];

/// The digest of the frozen test split (see [`test_digest`]). Changing a test item changes it; that is a deliberate
/// act recorded in the README, never a side effect of tuning.
pub const FROZEN_TEST_DIGEST: &str = "d39c6089d6050a63";

/// The documented example credentials that may appear literally (AWS's own documentation keys).
const DOCUMENTED_EXAMPLES: [&str; 2] = [
    "AKIAIOSFODNN7EXAMPLE",
    "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
];

/// The right verdict for an item. A judge may also defer, which is never a ground truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Truth {
    Allow,
    Redact,
    Deny,
}

impl Truth {
    pub const ALL: [Truth; 3] = [Truth::Allow, Truth::Redact, Truth::Deny];

    pub fn parse(s: &str) -> Option<Truth> {
        match s {
            "allow" => Some(Truth::Allow),
            "redact" => Some(Truth::Redact),
            "deny" => Some(Truth::Deny),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Truth::Allow => "allow",
            Truth::Redact => "redact",
            Truth::Deny => "deny",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Dev items are for tuning judges and prompts; test items are frozen and only scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    Dev,
    Test,
}

impl Split {
    pub fn parse(s: &str) -> Option<Split> {
        match s {
            "dev" => Some(Split::Dev),
            "test" => Some(Split::Test),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Split::Dev => "dev",
            Split::Test => "test",
        }
    }
}

/// One labelled item, with its placeholders expanded in `text` and `spans`.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub split: Split,
    pub category: String,
    /// The label of the room the message was posted under.
    pub label: String,
    /// The label of the readers it would cross to; strictly below `label`.
    pub to: String,
    pub verdict: Truth,
    /// The text as stored, placeholders and all.
    pub raw_text: String,
    /// The text a judge sees.
    pub text: String,
    pub rationale: String,
    /// The parts that must not cross (expanded). Empty exactly when the verdict is allow. A redaction that still
    /// contains one of these is a leak.
    pub spans: Vec<String>,
}

impl Item {
    pub fn is_injection(&self) -> bool {
        self.category.starts_with("injection")
    }
}

const FIELDS: [&str; 9] = [
    "id",
    "split",
    "category",
    "label",
    "to",
    "verdict",
    "text",
    "rationale",
    "spans",
];

fn field<'a>(o: &'a serde_json::Map<String, Value>, name: &str) -> Result<&'a str, String> {
    match o.get(name) {
        None => Err(format!("missing field \"{name}\"")),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s),
        Some(_) => Err(format!("field \"{name}\" must be a non-empty string")),
    }
}

/// Parse and check one line.
pub fn parse_line(line: &str) -> Result<Item, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    let o = v.as_object().ok_or("not a JSON object")?;
    if let Some(k) = o.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("unknown field \"{k}\""));
    }
    let id = field(o, "id")?.to_string();
    let split = Split::parse(field(o, "split")?).ok_or("split must be dev or test")?;
    let category = field(o, "category")?.to_string();
    if !CATEGORIES.contains(&category.as_str()) {
        return Err(format!("unknown category \"{category}\""));
    }
    let label = field(o, "label")?.to_string();
    let to = field(o, "to")?.to_string();
    for l in [&label, &to] {
        if !LABELS.contains(&l.as_str()) {
            return Err(format!("unknown label \"{l}\""));
        }
    }
    let chain = LabelChain::default();
    if chain.rank(&to) >= chain.rank(&label) {
        return Err(format!(
            "a crossing goes down: to ({to}) must be below label ({label})"
        ));
    }
    let verdict =
        Truth::parse(field(o, "verdict")?).ok_or("verdict must be allow, redact or deny")?;
    let raw_text = field(o, "text")?.to_string();
    let rationale = field(o, "rationale")?.to_string();
    let raw_spans: Vec<String> = match o.get("spans") {
        None => vec![],
        Some(Value::Array(a)) => a
            .iter()
            .map(|s| s.as_str().map(String::from).ok_or("spans must be strings"))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("spans must be an array of strings".into()),
    };
    let text = expand(&id, &raw_text)?;
    let spans = raw_spans
        .iter()
        .map(|s| expand(&id, s))
        .collect::<Result<Vec<_>, _>>()?;
    match (verdict, spans.is_empty()) {
        (Truth::Allow, false) => return Err("an allow item carries no spans".into()),
        (Truth::Redact | Truth::Deny, true) => {
            return Err("a redact or deny item names its spans (what must not cross)".into());
        }
        _ => {}
    }
    if let Some(s) = spans
        .iter()
        .find(|s| s.trim().is_empty() || !text.contains(*s))
    {
        return Err(format!("span {s:?} is not in the text"));
    }
    Ok(Item {
        id,
        split,
        category,
        label,
        to,
        verdict,
        raw_text,
        text,
        rationale,
        spans,
    })
}

/// Parse a whole file; blank lines are skipped. The first bad line is the error, with its number.
pub fn parse_corpus(text: &str) -> Result<Vec<Item>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(n, l)| parse_line(l).map_err(|e| format!("line {}: {e}", n + 1)))
        .collect()
}

/// What is wrong with a set of items that each parsed: today, a repeated id.
pub fn problems(items: &[Item]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    items
        .iter()
        .filter(|i| !seen.insert(i.id.as_str()))
        .map(|i| format!("id {} repeats", i.id))
        .collect()
}

/// Load and check a corpus file.
pub fn load(path: &std::path::Path) -> Result<Vec<Item>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let items = parse_corpus(&text)?;
    let p = problems(&items);
    if p.is_empty() {
        Ok(items)
    } else {
        Err(p.join("; "))
    }
}

/// The items of a split (or all), in file order, at most `limit`.
pub fn select(items: &[Item], split: Option<Split>, limit: Option<usize>) -> Vec<&Item> {
    items
        .iter()
        .filter(|i| split.is_none_or(|s| i.split == s))
        .take(limit.unwrap_or(usize::MAX))
        .collect()
}

pub fn default_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("judge-eval/corpus.jsonl")
}

pub fn default_scrub_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("judge-eval/scrub.txt")
}

/// FNV-1a, 64 bits: a stable hash with no dependency, for seeds and the test-split digest.
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A digest of every test item, in id order, over everything that defines it.
pub fn test_digest(items: &[Item]) -> u64 {
    let mut test: Vec<&Item> = items.iter().filter(|i| i.split == Split::Test).collect();
    test.sort_by(|a, b| a.id.cmp(&b.id));
    let mut buf = String::new();
    for i in test {
        for part in [
            i.id.as_str(),
            i.category.as_str(),
            i.label.as_str(),
            i.to.as_str(),
            i.verdict.word(),
            i.raw_text.as_str(),
            i.rationale.as_str(),
        ] {
            buf.push_str(part);
            buf.push('\u{1f}');
        }
        buf.push_str(&i.spans.join("\u{1f}"));
        buf.push('\u{1e}');
    }
    fnv(buf.as_bytes())
}

/// A xorshift64* stream seeded from a string.
struct Stream(u64);

impl Stream {
    fn new(seed: &str) -> Self {
        Stream(fnv(seed.as_bytes()) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn take(&mut self, alphabet: &str, n: usize) -> String {
        let a: Vec<char> = alphabet.chars().collect();
        (0..n)
            .map(|_| a[(self.next() % a.len() as u64) as usize])
            .collect()
    }
}

const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ALNUM: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const B64: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64URL: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const DIGITS: &str = "0123456789";
const HEX: &str = "0123456789abcdef";

/// The placeholder kinds and what each expands to. Secret kinds first, then look-alikes that are not secrets but
/// would trip a conservative scanner if stored literally.
pub const KINDS: [&str; 19] = [
    "aws",
    "aws-secret",
    "github",
    "github-pat",
    "slack",
    "stripe",
    "openai",
    "anthropic",
    "google",
    "jwt",
    "pem",
    "rsa",
    "npm",
    "hex32",
    "bearer",
    "password",
    "sri",
    "b64blob",
    "ssh-pub",
];

fn generate(kind: &str, s: &mut Stream) -> Option<String> {
    Some(match kind {
        "aws" => format!("AKIA{}", s.take("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567", 16)),
        "aws-secret" => s.take(B64, 40),
        "github" => format!("ghp_{}", s.take(ALNUM, 36)),
        "github-pat" => format!("github_pat_{}_{}", s.take(ALNUM, 22), s.take(ALNUM, 59)),
        "slack" => format!(
            "xoxb-{}-{}-{}",
            s.take(DIGITS, 12),
            s.take(DIGITS, 13),
            s.take(ALNUM, 24)
        ),
        "stripe" => format!("sk_live_{}", s.take(ALNUM, 24)),
        "openai" => format!("sk-proj-{}", s.take(B64URL, 48)),
        "anthropic" => format!("sk-ant-api03-{}", s.take(B64URL, 93)),
        "google" => format!("AIza{}", s.take(B64URL, 35)),
        "jwt" => format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ{}.{}",
            s.take(B64URL, 61),
            s.take(B64URL, 43)
        ),
        "pem" => format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA{}\n{}\n{}\n-----END OPENSSH PRIVATE KEY-----",
            s.take(B64, 46),
            s.take(B64, 70),
            s.take(B64, 52)
        ),
        "rsa" => format!(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA{}\n{}\n-----END RSA PRIVATE KEY-----",
            s.take(B64, 48),
            s.take(B64, 64)
        ),
        "npm" => format!("npm_{}", s.take(ALNUM, 36)),
        "hex32" => s.take(HEX, 32),
        "bearer" => s.take(B64URL, 40),
        "password" => {
            let core = s.take(ALNUM, 14);
            format!("{}{}{}", &core[..7], s.take("!#%*-_", 2), &core[7..])
        }
        "sri" => format!("sha384-{}", s.take(B64, 64)),
        "b64blob" => format!("{}{}", s.take(UPPER, 2), s.take(B64, 58)),
        "ssh-pub" => format!("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI{}", s.take(B64, 43)),
        _ => return None,
    })
}

/// Replace every `{{fake:KIND}}` or `{{fake:KIND:N}}` in `raw` with its generated string. The same id, kind and
/// index always give the same string; `{{fake:KIND}}` is index 1.
pub fn expand(id: &str, raw: &str) -> Result<String, String> {
    const OPEN: &str = "{{fake:";
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        let after = &rest[at + OPEN.len()..];
        let end = after
            .find("}}")
            .ok_or_else(|| format!("unclosed placeholder in {id}"))?;
        let inner = &after[..end];
        let (kind, n) = match inner.split_once(':') {
            None => (inner, 1),
            Some((k, n)) => (
                k,
                n.parse::<usize>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| format!("bad placeholder index in {id}: {inner}"))?,
            ),
        };
        let mut s = Stream::new(&format!("{id}|{kind}|{n}"));
        let value = generate(kind, &mut s)
            .ok_or_else(|| format!("unknown placeholder kind {kind:?} in {id}"))?;
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Why `text` looks like it holds a real credential, by a conservative check, or `None`. The documented AWS example
/// keys are allowed; everything the engine's own credential scanner matches is not, nor is a private-key header, a
/// Slack or npm token prefix, or any run of 32 or more token characters mixing upper case, lower case and digits.
/// Lower-case hex (hashes, digests, ids) passes.
pub fn looks_like_real_credential(text: &str) -> Option<&'static str> {
    let mut t = text.to_string();
    for ex in DOCUMENTED_EXAMPLES {
        t = t.replace(ex, " ");
    }
    if credential_kind(&t).is_some() {
        return Some("a credential shape the engine recognises");
    }
    if t.contains("PRIVATE KEY-----") {
        return Some("a private key header");
    }
    let b = t.as_bytes();
    for (i, _) in t.match_indices("xox") {
        if b.get(i + 3).is_some_and(|c| b"abposr".contains(c)) && b.get(i + 4) == Some(&b'-') {
            return Some("a Slack token prefix");
        }
    }
    let token = |c: char| c.is_ascii_alphanumeric() || "+/_=-".contains(c);
    for word in t.split(|c: char| !token(c)) {
        if word.starts_with("npm_") && word.len() >= 34 {
            return Some("an npm token");
        }
        let mixed = word.len() >= 32
            && word.chars().any(|c| c.is_ascii_uppercase())
            && word.chars().any(|c| c.is_ascii_lowercase())
            && word.chars().any(|c| c.is_ascii_digit());
        if mixed {
            return Some("a long mixed-case token");
        }
    }
    None
}

#[cfg(test)]
#[path = "corpus_tests.rs"]
mod tests;
