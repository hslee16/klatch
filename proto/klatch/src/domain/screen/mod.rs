//! The rules screen (#1413): the checks the default judge runs after the credential scanner, all local, no model, no
//! network, no new dependency. Each check is a hand-written scan that reads the text a bounded number of times
//! (linear in its length), so no input can make it backtrack; `MAX_SCREEN_BYTES` bounds what the new checks read.
//!
//! What it covers: the scrub list through lookalike letters, invisible characters, line breaks and reversal; personal
//! contact details and labelled identifiers (`pii`); text that addresses the screen to change its verdict
//! (`injection`); internal host names and private addresses; and a short list of confidentiality phrases. Some checks
//! depend on the labels (below). It never allows or redacts: a hit is a deny whose reason names the rule, never the
//! text. What it does not do is in SECURITY.md ("Channel-safety screening").

mod injection;
mod pii;

use super::LabelChain;

/// Text longer than this is not given to the new checks (the credential scanner and the scrub list still read all of
/// it). A message past the bound is held for the reviewer like any text the rules do not match.
pub const MAX_SCREEN_BYTES: usize = 64 * 1024;

/// Why the rules hold `text` crossing from `from` to `to`, or `None`. `terms` are the scrub list as (name, term).
/// The reason names a rule and never carries the text.
pub fn screen(
    text: &str,
    from: &str,
    to: &str,
    chain: &LabelChain,
    terms: &[(String, String)],
) -> Option<String> {
    let outward = chain.rank(to).is_none_or(|r| r == 0);
    let below_cap = !chain.dominates(to, chain.default_cap());
    if text.len() > MAX_SCREEN_BYTES {
        return None;
    }
    let sensitive_source = chain.rank(from).is_none() || chain.dominates(from, chain.default_cap());
    let fold = fold(text);
    let norm = fold.to_ascii_lowercase();
    let hit = |rule: &str| Some(format!("screen:{rule}"));
    if below_cap && let Some(name) = scrub_hit(&norm, terms) {
        return Some(format!("scrub:{name}"));
    }
    if injection::injection(&fold, &norm) {
        return hit("injection");
    }
    if pii::identifier(&fold, &norm) || pii::spoken_secret(&fold, &norm) {
        return hit("personal-identifier");
    }
    if outward && (pii::contact(&norm) || internal_host(&norm)) {
        return hit(if internal_host(&norm) {
            "internal-host"
        } else {
            "personal-contact"
        });
    }
    if (outward || sensitive_source) && confidential(&norm) {
        return hit("confidential-matter");
    }
    None
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

/// The name of the first scrub term found in the text as written, through the ways of writing it that the plain
/// substring test misses: lookalike letters, invisible characters, a line break or extra space between words, and the
/// text reversed (terms of six or more characters only, so a short term does not match by chance).
fn scrub_hit(norm: &str, terms: &[(String, String)]) -> Option<String> {
    let reversed: String = norm.chars().rev().collect();
    for (name, term) in terms {
        let wanted = fold(term).to_ascii_lowercase();
        if wanted.is_empty() {
            continue;
        }
        if norm.contains(&wanted) || (wanted.chars().count() >= 6 && reversed.contains(&wanted)) {
            return Some(name.clone());
        }
    }
    None
}

/// Labels that mark a name as inside a network when they are the last label (`db.internal`) or sit between two others
/// (`ci-04.corp.example`). `local` is left out: `profile.local` is as likely a config table as a host.
const INTERNAL_TLDS: &[&str] = &["internal", "corp", "lan", "intranet", "localdomain"];
const INTERNAL_LABELS: &[&str] = &[
    "corp", "internal", "intranet", "int", "lan", "priv", "intra",
];

fn internal_host(norm: &str) -> bool {
    norm.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
        .any(|token| {
            let host = token.trim_matches(|c| c == '.' || c == '-');
            let labels: Vec<&str> = host.split('.').collect();
            if labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
                return false;
            }
            private_address(&labels)
                || labels.last().is_some_and(|l| INTERNAL_TLDS.contains(l))
                || (labels.len() >= 3
                    && labels[1..labels.len() - 1]
                        .iter()
                        .any(|l| INTERNAL_LABELS.contains(l)))
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
/// Chosen to be rare in ordinary work chat; they will miss most private matter, which has no tell.
const CONFIDENTIAL: &[&str] = &[
    "layoff",
    "laid off",
    "redundancies",
    "severance",
    "reorg",
    "term sheet",
    "in talks to acquire",
    "embargo",
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
    "not announced yet",
    "hasn't announced",
    "has not been announced",
    "unannounced",
    "medical leave",
    "performance review",
    "improvement plan",
    "formal process",
    "background check",
    "conviction",
    "interviewing elsewhere",
    "legal says",
    "settlement",
    "patent claim",
    "unfixed",
    "unpatched",
];

fn confidential(norm: &str) -> bool {
    CONFIDENTIAL.iter().any(|p| norm.contains(p))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
