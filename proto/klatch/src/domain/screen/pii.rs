//! Personal data checks for the rules screen: contact details, labelled identifiers and a password written in
//! another language. All scans are linear in the text: each position is looked at a bounded number of times.

const ROLE_MAILBOXES: &[&str] = &[
    "support",
    "info",
    "noreply",
    "no-reply",
    "security",
    "press",
    "hello",
    "admin",
    "contact",
    "sales",
    "help",
    "abuse",
    "privacy",
    "legal",
    "git",
    "postmaster",
    "webmaster",
    "team",
    "docs",
];

/// A personal email address, a phone number or a street address, in the lower-cased text.
pub(super) fn contact(norm: &str) -> bool {
    email(norm.as_bytes()) || phone(norm.as_bytes()) || street_address(norm)
}

fn is_local(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn email(b: &[u8]) -> bool {
    for (at, _) in b.iter().enumerate().filter(|(_, c)| **c == b'@') {
        let mut s = at;
        while s > 0 && is_local(b[s - 1]) {
            s -= 1;
        }
        let local = &b[s..at];
        if local.is_empty() || !local.iter().any(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let mut e = at + 1;
        while e < b.len() && (b[e].is_ascii_alphanumeric() || matches!(b[e], b'.' | b'-')) {
            e += 1;
        }
        let domain = &b[at + 1..e];
        let domain = domain.strip_suffix(b".").unwrap_or(domain);
        let Some(dot) = domain.iter().rposition(|c| *c == b'.') else {
            continue;
        };
        let tld = &domain[dot + 1..];
        let role = std::str::from_utf8(local).is_ok_and(|l| ROLE_MAILBOXES.contains(&l));
        // `git@host:path` is a clone address, not a person.
        if dot > 0 && tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic) && !role {
            return true;
        }
    }
    false
}

fn digits(b: &[u8], i: usize, n: usize) -> bool {
    i + n <= b.len()
        && b[i..i + n].iter().all(u8::is_ascii_digit)
        && !b.get(i + n).is_some_and(u8::is_ascii_digit)
}

fn starts_token(b: &[u8], i: usize) -> bool {
    i == 0
        || !(b[i - 1].is_ascii_alphanumeric()
            || matches!(b[i - 1], b'-' | b'.' | b'_' | b'#' | b'/'))
}

fn ends_token(b: &[u8], i: usize) -> bool {
    !b.get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
}

/// `555-0142`, `555-010-4477`, `(555) 010-4477` and `+1 555 010 4488`.
fn phone(b: &[u8]) -> bool {
    for i in 0..b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let after_paren = i > 0 && b[i - 1] == b'(';
            if (starts_token(b, i) || after_paren) && local_number(b, i, after_paren) {
                return true;
            }
        } else if b[i] == b'+' && starts_token(b, i) && international(b, i) {
            return true;
        }
    }
    false
}

fn local_number(b: &[u8], i: usize, after_paren: bool) -> bool {
    if after_paren {
        if !(digits(b, i, 3) && b.get(i + 3) == Some(&b')')) {
            return false;
        }
        let j = i + 4 + usize::from(b.get(i + 4) == Some(&b' '));
        return digits(b, j, 3)
            && matches!(b.get(j + 3), Some(b'-' | b' '))
            && digits(b, j + 4, 4)
            && ends_token(b, j + 8);
    }
    if !(digits(b, i, 3) && b.get(i + 3) == Some(&b'-')) {
        return false;
    }
    (digits(b, i + 4, 4) && ends_token(b, i + 8))
        || (digits(b, i + 4, 3)
            && b.get(i + 7) == Some(&b'-')
            && digits(b, i + 8, 4)
            && ends_token(b, i + 12))
}

/// A `+` and a country code, then at least nine digits among spaces, hyphens, dots and brackets.
fn international(b: &[u8], i: usize) -> bool {
    let mut n = 0;
    let mut j = i + 1;
    while j < b.len()
        && j - i <= 24
        && (b[j].is_ascii_digit() || matches!(b[j], b' ' | b'-' | b'.' | b'(' | b')'))
    {
        n += usize::from(b[j].is_ascii_digit());
        j += 1;
    }
    j > i + 1
        && b[i + 1].is_ascii_digit()
        && n >= 9
        && !b.get(j).is_some_and(u8::is_ascii_alphanumeric)
}

const STREET_SUFFIX: &[&str] = &[
    "road",
    "street",
    "avenue",
    "lane",
    "boulevard",
    "rd",
    "ave",
    "blvd",
    "straße",
    "strasse",
    "gasse",
];
const STREET_PREFIX: &[&str] = &["calle", "rua", "rue", "avenida", "paseo"];

fn word(t: &str) -> &str {
    t.trim_matches(|c: char| !c.is_alphanumeric())
}

fn is_number(t: &str) -> bool {
    let n = word(t);
    let d = n.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    !d.is_empty() && d.len() <= 5 && n.len() <= d.len() + 1 && d.chars().all(|c| c.is_ascii_digit())
}

fn is_name_word(t: &str) -> bool {
    let w = word(t);
    w.chars().count() >= 2 && w.chars().all(|c| c.is_alphabetic())
}

/// `22 Birch Hollow Road` and `Calle Ejemplo 14`: a house number and one to three words around a street word.
fn street_address(norm: &str) -> bool {
    let toks: Vec<&str> = norm
        .split(|c: char| c == ' ' || "'\"()[]{}".contains(c))
        .filter(|t| !t.is_empty())
        .collect();
    for (k, t) in toks.iter().enumerate() {
        if is_number(t) {
            let end = toks.len().min(k + 5);
            for (m, s) in toks[k + 1..end].iter().enumerate() {
                if !is_name_word(s) {
                    break;
                }
                if m >= 1 && STREET_SUFFIX.contains(&word(s)) {
                    return true;
                }
            }
        }
        if STREET_PREFIX.contains(&word(t)) {
            let end = toks.len().min(k + 5);
            for (m, s) in toks[k + 1..end].iter().enumerate() {
                if is_number(s) && m >= 1 {
                    return true;
                }
                if !is_name_word(s) {
                    break;
                }
            }
        }
    }
    false
}

const ID_LABELS: &[&str] = &[
    "ssn",
    "dl",
    "social security",
    "national id",
    "national_id",
    "passport",
    "driver's licen",
    "drivers licen",
    "driving licen",
    "tax id",
    "taxpayer id",
    "iban",
    "routing number",
    "account number",
    "account ending",
    "card ending",
    "card number",
    "card no",
];
const BIRTH_LABELS: &[&str] = &[
    "date of birth",
    "date_of_birth",
    "dob",
    "born",
    "birthday",
    "geburtsdatum",
];

fn label_before_ok(norm: &str, at: usize) -> bool {
    at == 0 || !norm.as_bytes()[at - 1].is_ascii_alphanumeric()
}

/// The `n` bytes after `at`, cut back to a character boundary.
fn window(s: &str, at: usize, n: usize) -> &str {
    let mut end = s.len().min(at + n);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[at..end]
}

fn digit_count(s: &str) -> usize {
    s.bytes().filter(u8::is_ascii_digit).count()
}

/// A government, bank or card identifier next to its label, a date of birth, a bare US social security number, an
/// IBAN, or a card number that passes the Luhn check.
pub(super) fn identifier(fold: &str, norm: &str) -> bool {
    let labelled = |labels: &[&str], ok: &dyn Fn(&str) -> bool| {
        labels.iter().any(|l| {
            norm.match_indices(l).any(|(at, m)| {
                let end = at + m.len();
                // a short label (`dl`, `ssn`) must be a word of its own
                let whole = l.len() > 4
                    || !norm
                        .as_bytes()
                        .get(end)
                        .is_some_and(u8::is_ascii_alphanumeric);
                label_before_ok(norm, at) && whole && ok(window(norm, end, 28))
            })
        })
    };
    labelled(ID_LABELS, &|w| digit_count(w) >= 4)
        || labelled(BIRTH_LABELS, &has_date)
        || ssn(norm.as_bytes())
        || iban(fold)
        || card(norm.as_bytes())
}

const MONTHS: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec", "janv",
    "févr", "enero", "märz", "juni", "juli",
];

/// A day-month-year or year-month-day date: `1990-06-17`, `17/06/1990`, `17.06.1990`, `June 17, 1990`.
fn has_date(w: &str) -> bool {
    let b = w.as_bytes();
    let sep = |i: usize| matches!(b.get(i), Some(b'-' | b'/' | b'.'));
    // the length of a one or two digit number that starts at `i` and is followed by a separator
    let short = |i: usize| (1..=2).find(|n| digits(b, i, *n) && sep(i + n));
    let year = |i: usize| digits(b, i, 4) && matches!(&b[i..i + 2], b"19" | b"20");
    for i in (0..b.len()).filter(|i| *i == 0 || !b[i - 1].is_ascii_digit()) {
        if year(i) && sep(i + 4) && (1..=2).any(|n| digits(b, i + 5, n)) {
            return true;
        }
        if let Some(n) = short(i)
            && let Some(m) = short(i + n + 1)
            && year(i + n + m + 2)
        {
            return true;
        }
    }
    let has_year = (0..b.len()).any(|i| (i == 0 || !b[i - 1].is_ascii_digit()) && year(i));
    has_year && MONTHS.iter().any(|m| w.contains(m))
}

fn ssn(b: &[u8]) -> bool {
    (0..b.len()).any(|i| {
        starts_token(b, i)
            && digits(b, i, 3)
            && b.get(i + 3) == Some(&b'-')
            && digits(b, i + 4, 2)
            && b.get(i + 6) == Some(&b'-')
            && digits(b, i + 7, 4)
            && ends_token(b, i + 11)
    })
}

/// Two capital letters, two digits and 11 to 30 capitals or digits, with at least ten digits: an IBAN shape that a hash
/// written in lower case or a plain word does not have.
fn iban(fold: &str) -> bool {
    fold.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| {
        let b = t.as_bytes();
        (15..=34).contains(&b.len())
            && b[..2].iter().all(u8::is_ascii_uppercase)
            && b[2..4].iter().all(u8::is_ascii_digit)
            && b[4..]
                .iter()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            && b.iter().filter(|c| c.is_ascii_digit()).count() >= 10
    })
}

/// 13 to 19 digits, grouped by single spaces or hyphens or not, that pass the Luhn check.
fn card(b: &[u8]) -> bool {
    for i in 0..b.len() {
        if !b[i].is_ascii_digit()
            || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'-'))
        {
            continue;
        }
        let mut ds = Vec::with_capacity(19);
        let mut j = i;
        while j < b.len() && ds.len() < 20 {
            if b[j].is_ascii_digit() {
                ds.push(b[j] - b'0');
            } else if !(matches!(b[j], b' ' | b'-')
                && b.get(j + 1).is_some_and(u8::is_ascii_digit)
                && !ds.is_empty())
            {
                break;
            }
            j += 1;
        }
        if (13..=19).contains(&ds.len())
            && !b.get(j).is_some_and(u8::is_ascii_alphanumeric)
            && luhn(&ds)
        {
            return true;
        }
    }
    false
}

fn luhn(ds: &[u8]) -> bool {
    let sum: u32 = ds
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            let d = u32::from(*d);
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 { x - 9 } else { x }
            } else {
                d
            }
        })
        .sum();
    sum % 10 == 0
}

const PASSWORD_WORDS: &[&str] = &[
    "密码",
    "密碼",
    "口令",
    "パスワード",
    "contraseña",
    "mot de passe",
    "passwort",
    "kennwort",
    "senha",
    "пароль",
    "passphrase",
];

/// A password word (in a language the credential scanner has no label for) followed within a few characters by a
/// value that has three of the four classes: upper case, lower case, digit, symbol. The scanner does the English labels.
pub(super) fn spoken_secret(fold: &str, norm: &str) -> bool {
    PASSWORD_WORDS.iter().any(|w| {
        norm.match_indices(w).any(|(at, m)| {
            window(fold, at + m.len(), 40)
                .split(|c: char| c.is_whitespace() || "，。；：、,;:=（）()[]<>\"'「」".contains(c))
                .any(looks_like_value)
        })
    })
}

fn looks_like_value(t: &str) -> bool {
    let n = t.chars().count();
    let class = |f: fn(&char) -> bool| usize::from(t.chars().any(|c| f(&c)));
    n >= 8
        && t.is_ascii()
        && class(char::is_ascii_uppercase)
            + class(char::is_ascii_lowercase)
            + class(char::is_ascii_digit)
            + class(char::is_ascii_punctuation)
            >= 3
        && !t.contains("${")
        && !t.contains("***")
}
