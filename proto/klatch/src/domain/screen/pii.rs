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
pub(super) fn contact(fold: &str, norm: &str) -> bool {
    email(norm.as_bytes()) || phone(norm.as_bytes()) || street_address(fold, norm)
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
        let role = std::str::from_utf8(local).is_ok_and(|l| ROLE_MAILBOXES.contains(&l))
            || domain.ends_with(b"noreply.github.com")
            || git_metadata_before(b, s);
        // `git@host:path` is a clone address, not a person.
        if dot > 0 && tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic) && !role {
            return true;
        }
    }
    false
}

const GIT_TRAILERS: &[&str] = &[
    "signed-off-by",
    "author:",
    "co-authored-by",
    "committer:",
    "reviewed-by",
    "acked-by",
    "tested-by",
    "reported-by",
];

/// Whether the address at `at` follows a commit trailer or an `Author:` line, which is metadata, not a person's contact.
fn git_metadata_before(b: &[u8], at: usize) -> bool {
    let from = at.saturating_sub(80);
    let before = String::from_utf8_lossy(&b[from..at]);
    // only the stretch after the last line break or `>` that closes an earlier address counts
    let tail = before.rsplit(['\n', '>']).next().unwrap_or("");
    GIT_TRAILERS.iter().any(|t| tail.contains(t))
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

/// `555-0142`, `555-010-4477`, `(555) 010-4477` and `+1 555 010 4488`, unless the numbers read as a range or a measure
/// (`lines 120-1450`, `250-1000 ms`).
fn phone(b: &[u8]) -> bool {
    for i in 0..b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let after_paren = i > 0 && b[i - 1] == b'(';
            if (starts_token(b, i) || after_paren)
                && let Some(end) = local_number(b, i, after_paren)
                && !range_context(b, i, end)
            {
                return true;
            }
        } else if b[i] == b'+' && starts_token(b, i) && international(b, i) {
            return true;
        }
    }
    false
}

const RANGE_BEFORE: &[&str] = &[
    "lines", "line", "range", "ranges", "between", "from", "bytes", "offset", "rows", "pages",
    "page", "ports", "port", "ids", "backoff", "timeout", "size", "sizes", "items", "columns",
    "chars", "step", "steps", "ms",
];
const RANGE_AFTER: &[&str] = &[
    "ms", "s", "sec", "secs", "seconds", "kb", "mb", "gb", "kib", "mib", "px", "bytes", "lines",
    "rows", "items", "ports", "hz", "min", "minutes", "chars", "%", "x",
];

fn word_before(b: &[u8], i: usize) -> String {
    let mut e = i;
    while e > 0 && matches!(b[e - 1], b' ' | b':' | b'=') {
        e -= 1;
    }
    let mut s = e;
    while s > 0 && b[s - 1].is_ascii_alphabetic() {
        s -= 1;
    }
    String::from_utf8_lossy(&b[s..e]).into_owned()
}

fn word_after(b: &[u8], i: usize) -> String {
    let mut s = i;
    while s < b.len() && b[s] == b' ' {
        s += 1;
    }
    let mut e = s;
    while e < b.len() && (b[e].is_ascii_alphabetic() || b[e] == b'%') {
        e += 1;
    }
    String::from_utf8_lossy(&b[s..e]).into_owned()
}

fn range_context(b: &[u8], i: usize, end: usize) -> bool {
    RANGE_BEFORE.contains(&word_before(b, i).as_str())
        || RANGE_AFTER.contains(&word_after(b, end).as_str())
}

/// The index just past a local number that starts at `i`, if there is one.
fn local_number(b: &[u8], i: usize, after_paren: bool) -> Option<usize> {
    if after_paren {
        if !(digits(b, i, 3) && b.get(i + 3) == Some(&b')')) {
            return None;
        }
        let j = i + 4 + usize::from(b.get(i + 4) == Some(&b' '));
        return (digits(b, j, 3)
            && matches!(b.get(j + 3), Some(b'-' | b' '))
            && digits(b, j + 4, 4)
            && ends_token(b, j + 8))
        .then_some(j + 8);
    }
    if !(digits(b, i, 3) && b.get(i + 3) == Some(&b'-')) {
        return None;
    }
    if digits(b, i + 4, 4) && ends_token(b, i + 8) {
        return Some(i + 8);
    }
    (digits(b, i + 4, 3)
        && b.get(i + 7) == Some(&b'-')
        && digits(b, i + 8, 4)
        && ends_token(b, i + 12))
    .then_some(i + 12)
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
    // the span ends at its last digit: a space or a full stop after the number is not part of it
    let mut e = j;
    while e > i + 1 && !b[e - 1].is_ascii_digit() {
        e -= 1;
    }
    e > i + 1
        && b[i + 1].is_ascii_digit()
        && n >= 9
        && !b.get(e).is_some_and(u8::is_ascii_alphanumeric)
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

const NOT_A_HOUSE_NUMBER: &[&str] = &[
    "step", "steps", "line", "lines", "page", "pages", "item", "section", "chapter", "part",
    "phase", "round", "version", "v", "no", "number", "row", "rows", "top", "last", "first",
];

fn capitalised(t: &str) -> bool {
    word(t).chars().next().is_some_and(char::is_uppercase)
}

/// `22 Birch Hollow Road` and `Calle Ejemplo 14`: a house number and one to three capitalised words around a street
/// word (`step 2 down the lane` is not one).
fn street_address(fold: &str, norm: &str) -> bool {
    let cut = |c: char| c == ' ' || "'\"()[]{}".contains(c);
    let toks: Vec<&str> = norm.split(cut).filter(|t| !t.is_empty()).collect();
    let orig: Vec<&str> = fold.split(cut).filter(|t| !t.is_empty()).collect();
    for (k, t) in toks.iter().enumerate() {
        if is_number(t) && !(k > 0 && NOT_A_HOUSE_NUMBER.contains(&word(toks[k - 1]))) {
            let end = toks.len().min(k + 5);
            for (m, s) in toks[k + 1..end].iter().enumerate() {
                if !is_name_word(s) || !capitalised(orig[k + 1 + m]) {
                    break;
                }
                if m >= 1 && STREET_SUFFIX.contains(&word(s)) {
                    return true;
                }
            }
        }
        if STREET_PREFIX.contains(&word(t)) && capitalised(orig[k]) {
            let end = toks.len().min(k + 5);
            for (m, s) in toks[k + 1..end].iter().enumerate() {
                if is_number(s) && m >= 1 {
                    return true;
                }
                if !is_name_word(s) || !capitalised(orig[k + 1 + m]) {
                    break;
                }
            }
        }
    }
    false
}

const ID_LABELS: &[&str] = &[
    "ssn",
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
        || dl_value(norm)
        || labelled(BIRTH_LABELS, &has_date)
        || ssn(norm.as_bytes())
        || iban(fold)
        || card(norm.as_bytes())
}

/// `DL K204-1187-5532`: the word `dl` on its own, then a licence-shaped value (eight or more characters, six digits
/// or more and a letter). `dl speed 1234 kb/s` is not one.
fn dl_value(norm: &str) -> bool {
    norm.match_indices("dl").any(|(at, m)| {
        let end = at + m.len();
        label_before_ok(norm, at)
            && !norm
                .as_bytes()
                .get(end)
                .is_some_and(u8::is_ascii_alphanumeric)
            && window(norm, end, 24)
                .trim_start_matches([' ', ':', '#', '=', '.', '('])
                .split([' ', ',', ')', ';'])
                .next()
                .is_some_and(|v| {
                    v.len() >= 8
                        && digit_count(v) >= 6
                        && v.bytes().any(|c| c.is_ascii_alphabetic())
                        && v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                })
    })
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

/// Length of an IBAN by country code (the registry's list).
const IBAN_LENGTHS: &str = "AD24 AE23 AL28 AT20 AZ28 BA20 BE16 BG22 BH22 BR29 BY28 CH21 CR22 CY28 CZ24 DE22 DK18 DO28 \
EE20 EG29 ES24 FI18 FO18 FR27 GB22 GE22 GI23 GL18 GR27 GT28 HR21 HU28 IE22 IL23 IQ23 IS26 IT27 JO30 KW30 KZ20 LB28 \
LC32 LI21 LT20 LU20 LV21 LY25 MC27 MD24 ME22 MK19 MR27 MT31 MU30 NL18 NO15 PK24 PL28 PS29 PT25 QA29 RO24 RS22 SA24 \
SC31 SE24 SI19 SK24 SM27 ST25 SV28 TL23 TN24 TR26 UA29 VA22 VG24 XK20";

/// An IBAN: a registered country code, two digits, and the length that country uses, capitals and digits only, with at
/// least ten digits. A hash in capitals (`BA12C4E5...`) has the wrong length for its country.
fn iban(fold: &str) -> bool {
    fold.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| {
        let b = t.as_bytes();
        b.len() >= 15
            && b[..2].iter().all(u8::is_ascii_uppercase)
            && b[2..4].iter().all(u8::is_ascii_digit)
            && b[4..]
                .iter()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            && b.iter().filter(|c| c.is_ascii_digit()).count() >= 10
            && IBAN_LENGTHS
                .split_whitespace()
                .any(|e| e.starts_with(&t[..2]) && e[2..].parse() == Ok(b.len()))
    })
}

/// A card number: an issuer prefix (Visa, Mastercard, Amex, Discover, Diners, JCB, UnionPay), digits in one run or in
/// a card grouping (4-4-4-4, 4-4-4-4-3, 4-6-5, 4-6-4) joined by single spaces or hyphens, not all one digit, and a
/// passing Luhn check. A long id or a list of small numbers is not one.
fn card(b: &[u8]) -> bool {
    for i in 0..b.len() {
        if !b[i].is_ascii_digit()
            || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'-'))
        {
            continue;
        }
        let (mut groups, mut ds, mut j) = (Vec::new(), Vec::with_capacity(19), i);
        loop {
            let s = j;
            while j < b.len() && b[j].is_ascii_digit() && ds.len() < 20 {
                ds.push(b[j] - b'0');
                j += 1;
            }
            groups.push(j - s);
            if groups.len() < 6
                && matches!(b.get(j), Some(b' ' | b'-'))
                && b.get(j + 1).is_some_and(u8::is_ascii_digit)
            {
                j += 1;
            } else {
                break;
            }
        }
        // a card grouping stands alone: a run of four-digit values inside a longer list of numbers is data
        let alone = !(i >= 2 && matches!(b[i - 1], b' ' | b'-') && b[i - 2].is_ascii_digit())
            && !(matches!(b.get(j), Some(b' ' | b'-'))
                && b.get(j + 1).is_some_and(u8::is_ascii_digit));
        let shaped = match groups.as_slice() {
            [n] => (13..=19).contains(n),
            g => alone && matches!(g, [4, 4, 4, 4] | [4, 4, 4, 4, 3] | [4, 6, 5] | [4, 6, 4]),
        };
        if shaped
            && !b.get(j).is_some_and(u8::is_ascii_alphanumeric)
            && ds.iter().any(|d| *d != ds[0])
            && issuer(&ds)
            && luhn(&ds)
        {
            return true;
        }
    }
    false
}

/// Whether the leading digits are in a card issuer's range for a number of this length.
fn issuer(ds: &[u8]) -> bool {
    let n = ds.len();
    let p = |k: usize| ds[..k].iter().fold(0u32, |a, d| a * 10 + u32::from(*d));
    (p(1) == 4 && matches!(n, 13 | 16 | 19))
        || ((51..=55).contains(&p(2)) || (2221..=2720).contains(&p(4))) && n == 16
        || (matches!(p(2), 34 | 37) && n == 15)
        || ((p(4) == 6011 || p(2) == 65 || (644..=649).contains(&p(3)) || p(2) == 62)
            && (16..=19).contains(&n))
        || ((3528..=3589).contains(&p(4)) && (16..=19).contains(&n))
        || (((300..=305).contains(&p(3)) || matches!(p(2), 36 | 38)) && (14..=19).contains(&n))
}

pub(super) fn luhn(ds: &[u8]) -> bool {
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
    sum.is_multiple_of(10)
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
        && !t.contains('/')
        && !t.contains("${")
        && !t.contains("***")
}
