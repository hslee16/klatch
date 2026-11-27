//! The rules screen (#1413): the checks the default judge runs after the credential scanner, all local, no model, no
//! network, no new dependency. Each check is a hand-written scan that reads the text a bounded number of times
//! (linear in its length), so no input can make it backtrack; `MAX_SCREEN_BYTES` bounds what the new checks read.
//!
//! What it covers: the scrub list through lookalike letters, invisible characters, line breaks and reversal; personal
//! contact details and labelled identifiers (`pii`); text that addresses the screen to change its verdict
//! (`injection`); internal host names and private addresses; and a short list of confidentiality phrases. Some checks
//! depend on the labels (below). It never allows or redacts. A scrub term found as written is a deny; every other hit
//! is a hold (the judge defers with a note that names the rule, never the text), so the item stays for the reviewer or a
//! model judge. What it does not do is in SECURITY.md ("Channel-safety screening").

mod injection;
mod pii;

use super::LabelChain;

/// Text longer than this is not given to the new checks (the credential scanner and the scrub list still read all of
/// it). A message past the bound is held for the reviewer like any text the rules do not match.
pub const MAX_SCREEN_BYTES: usize = 64 * 1024;

/// What the screen says about a crossing. `Deny` is final (the scrub list, a floor); `Hold` flags the text for the
/// reviewer or a model judge to decide, with a reason that names the rule and never carries the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screened {
    Deny(String),
    Hold(String),
}

impl Screened {
    pub fn reason(&self) -> &str {
        match self {
            Screened::Deny(r) | Screened::Hold(r) => r,
        }
    }
}

/// Why the rules flag `text` crossing from `from` to `to`, or `None`. `terms` are the scrub list as (name, term).
/// The reason names a rule and never carries the text. Only a scrub term found as written (through lookalike letters,
/// invisible characters and line breaks too) is a `Deny`; every heuristic family, and a term found only reversed, is a
/// `Hold`: the item stays queued for the reviewer or a model judge, which decides.
pub fn screen(
    text: &str,
    _from: &str,
    to: &str,
    chain: &LabelChain,
    terms: &[(String, String)],
) -> Option<Screened> {
    let outward = chain.rank(to).is_none_or(|r| r == 0);
    let below_cap = !chain.dominates(to, chain.default_cap());
    if text.len() > MAX_SCREEN_BYTES {
        return None;
    }
    let fold = fold(text);
    let norm = fold.to_ascii_lowercase();
    let hold = |rule: &str| Some(Screened::Hold(format!("screen:{rule}")));
    let mut reversed_hit = None;
    if below_cap {
        match scrub_hit(&norm, terms) {
            Some((name, false)) => return Some(Screened::Deny(format!("scrub:{name}"))),
            Some((name, true)) => {
                reversed_hit = Some(Screened::Hold(format!("scrub:{name}:reversed")))
            }
            None => {}
        }
    }
    if injection::injection(&fold, &norm) {
        return hold("injection");
    }
    if pii::identifier(&fold, &norm) || pii::spoken_secret(&fold, &norm) {
        return hold("personal-identifier");
    }
    if outward {
        if internal_host(&norm) {
            return hold("internal-host");
        }
        if pii::contact(&fold, &norm) {
            return hold("personal-contact");
        }
        if confidential(&norm) {
            return hold("confidential-matter");
        }
    }
    reversed_hit
}

/// The text without invisible characters, full-width forms folded to ASCII, a lookalike Cyrillic or Greek letter inside a
/// word that also has Latin letters mapped to the Latin one, and white space collapsed to single spaces. Case is kept.
pub(super) fn fold(text: &str) -> String {
    let clean = super::credentials::strip_invisible(text);
    let mut out = String::with_capacity(clean.len());
    for (i, word) in clean.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let latin = word.chars().any(|c| c.is_ascii_alphabetic());
        for c in word.chars() {
            out.push(match c {
                '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
                _ if latin => lookalike(c),
                _ => c,
            });
        }
    }
    out
}

fn lookalike(c: char) -> char {
    match c {
        'а' | 'А' | 'α' | 'Α' => 'a',
        'в' | 'В' | 'β' | 'Β' => 'b',
        'с' | 'С' | 'ϲ' => 'c',
        'ԁ' => 'd',
        'е' | 'Е' | 'ε' | 'Ε' => 'e',
        'ɡ' => 'g',
        'һ' | 'Н' | 'Η' => 'h',
        'і' | 'І' | 'ı' | 'ι' | 'Ι' => 'i',
        'ј' | 'Ј' => 'j',
        'к' | 'К' | 'κ' | 'Κ' => 'k',
        'ӏ' | 'Ӏ' => 'l',
        'м' | 'М' | 'Μ' => 'm',
        'ո' | 'η' => 'n',
        'о' | 'О' | 'ο' | 'Ο' => 'o',
        'р' | 'Р' | 'ρ' | 'Ρ' => 'p',
        'ѕ' | 'Ѕ' => 's',
        'т' | 'Т' | 'τ' | 'Τ' => 't',
        'υ' => 'u',
        'ν' => 'v',
        'х' | 'Х' | 'χ' | 'Χ' => 'x',
        'у' | 'У' | 'γ' => 'y',
        'Ζ' => 'z',
        _ => c,
    }
}

/// The first scrub term found in the text as written, through the ways of writing it that the plain substring test
/// misses (lookalike letters, invisible characters, a line break or extra space between words), as `(name, false)`;
/// failing that, one found only reversed, as a whole word and only for terms of six or more characters, as
/// `(name, true)`. A term that is itself a real word backwards (`reward`, `drawer`) matches that word, so reversal is a
/// hold, never a deny. Forward matching is a plain substring match, as the floor's is.
fn scrub_hit(norm: &str, terms: &[(String, String)]) -> Option<(String, bool)> {
    let reversed: String = norm.chars().rev().collect();
    let mut backwards = None;
    for (name, term) in terms {
        let wanted = fold(term).to_ascii_lowercase();
        if wanted.is_empty() {
            continue;
        }
        if norm.contains(&wanted) {
            return Some((name.clone(), false));
        }
        if backwards.is_none() && wanted.chars().count() >= 6 && whole_word(&reversed, &wanted) {
            backwards = Some((name.clone(), true));
        }
    }
    backwards
}

/// Whether `needle` occurs in `hay` with no letter or digit on either side.
fn whole_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(at, m)| {
        let before = hay[..at].chars().next_back();
        let after = hay[at + m.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Labels that mark a name as inside a network when they are the last label (`db.internal`) or sit between two others
/// (`ci-04.corp.example`). `local`, `priv` and `intra` are left out (`profile.local` is a config table, `x.priv.pem` a
/// file), as are package paths and file names (`jdk.internal.reflect...`, `id_ed25519.priv.pem`).
const INTERNAL_TLDS: &[&str] = &["internal", "corp", "lan", "intranet", "localdomain"];
const INTERNAL_LABELS: &[&str] = &["corp", "internal", "intranet", "int", "lan"];
const PACKAGE_ROOTS: &[&str] = &[
    "java", "javax", "jdk", "sun", "com", "org", "io", "net", "android", "kotlin", "scala", "std",
    "core", "alloc", "crate", "self", "super",
];
const FILE_EXTENSIONS: &[&str] = &[
    "pem", "key", "crt", "cer", "json", "yaml", "yml", "txt", "log", "toml", "md", "rs", "py",
    "js", "ts", "sh", "conf", "cfg", "lock", "bak", "csv", "html", "css", "xml", "ini", "env",
    "pub", "sig", "tar", "gz", "zip",
];

fn internal_host(norm: &str) -> bool {
    norm.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
        .any(|token| {
            let host = token.trim_matches(|c| c == '.' || c == '-');
            let labels: Vec<&str> = host.split('.').collect();
            if labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
                return false;
            }
            if private_address(&labels) {
                return true;
            }
            let last = labels[labels.len() - 1];
            let package = PACKAGE_ROOTS.contains(&labels[0]) || FILE_EXTENSIONS.contains(&last);
            let word_like = last.len() >= 2 && last.bytes().all(|c| c.is_ascii_lowercase());
            !package
                && word_like
                && (INTERNAL_TLDS.contains(&last)
                    || (labels.len() >= 3
                        && last.len() <= 8
                        && labels[1..labels.len() - 1]
                            .iter()
                            .any(|l| INTERNAL_LABELS.contains(l))))
        })
}

fn private_address(labels: &[&str]) -> bool {
    let nums: Vec<u16> = labels.iter().filter_map(|l| l.parse().ok()).collect();
    nums.len() == 4
        && labels.len() == 4
        && nums.iter().all(|n| *n <= 255)
        && (nums[0] == 10
            || (nums[0] == 192 && nums[1] == 168)
            || (nums[0] == 172 && (16..=31).contains(&nums[1])))
}

/// Phrases that mark a message as one its sender wants kept inside, or that name a matter that is not for outsiders.
/// Kept to ones that are rare in engineering talk (`reorg`, `performance review`, `settlement`, `unpatched` were
/// dropped after the review of #1734: they are everyday words there); they will miss most private matter, which has
/// no tell.
const CONFIDENTIAL: &[&str] = &[
    "layoff",
    "laid off",
    "redundancies",
    "term sheet",
    "in talks to acquire",
    "embargoed",
    "under nda",
    "off the record",
    "mention it outside",
    "mention this outside",
    "don't tell the",
    "do not tell the",
    "not telling",
    "keep it quiet",
    "keep this quiet",
    "keep it between us",
    "keep this between",
    "hasn't told",
    "has not told",
    "not yet announced",
    "hasn't announced",
    "has not been announced",
    "medical leave",
    "improvement plan",
    "interviewing elsewhere",
    "patent claim",
];

fn confidential(norm: &str) -> bool {
    CONFIDENTIAL.iter().any(|p| norm.contains(p))
}

#[cfg(test)]
mod benign;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tests_review.rs"]
mod tests_review;
