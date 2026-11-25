//! A small, bounded DER reader for the two fields of a CRL that rustls does not expose (#1614, #1491): the issuer name
//! and the validity times (`thisUpdate`, `nextUpdate`). No dependency, no unsafe code, and the rules that
//! make no input slow or large: definite minimal lengths of at most four bytes only, no recursion (a fixed walk of
//! `tbsCertList`, never into a child), nothing allocated, every read bounded by the slice it is given. Any other input is
//! `None`, which the caller treats as "not a valid X.509 CRL". Names no rustls type.
//!
//! **Times** follow RFC 5280 section 5.1.2.4 and DER, strictly: `UTCTime` is exactly `YYMMDDHHMMSSZ` (13 bytes; `YY` of
//! 50 or more is 19YY, else 20YY) and `GeneralizedTime` exactly `YYYYMMDDHHMMSSZ` (15 bytes); `Z` is required (no offset,
//! no lower case), no fractional seconds, digits only (no sign, no space), month 1 to 12, day 1 to the length of that
//! month (leap years counted), hour under 24, minute under 60 and second under 60 (no leap second). A
//! `GeneralizedTime` for a year before 2050 is accepted although RFC 5280 asks for `UTCTime` there: refusing a CRL a
//! CA made slightly off-profile would lock the operator out of a list webpki itself reads.

/// One DER element: its tag, its content and what follows. Definite, minimal lengths of at most four bytes only; a high
/// tag number, an indefinite length, a length that runs past the input and a length not in its shortest form are all
/// `None`. Nothing recurses and nothing allocates, so no input makes this slow or large.
pub fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n || rest[0] == 0 {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |a, b| (a << 8) | usize::from(*b));
        if len < 0x80 {
            return None;
        }
        (len, &rest[n..])
    };
    (len <= rest.len()).then(|| (tag, &rest[..len], &rest[len..]))
}

const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
/// `[0]` constructed: the `crlExtensions` of a `tbsCertList`.
const EXTENSIONS: u8 = 0xa0;

/// The issuer name (its DER content) of a CRL and the bytes of `tbsCertList` after it: `CertificateList ::= SEQUENCE {
/// tbsCertList SEQUENCE { version INTEGER OPTIONAL, signature AlgorithmIdentifier, issuer Name, thisUpdate Time,
/// nextUpdate Time OPTIONAL, ... }, ... }`. rustls-webpki compares issuers as these bytes.
fn after_issuer(crl: &[u8]) -> Option<(&[u8], &[u8])> {
    let (list_tag, list, after) = tlv(crl)?;
    let (tbs_tag, tbs, _) = tlv(list)?;
    if list_tag != SEQUENCE || tbs_tag != SEQUENCE || !after.is_empty() {
        return None;
    }
    let (mut tag, _, mut rest) = tlv(tbs)?;
    if tag == INTEGER {
        // the optional version; the signature algorithm is the element after it
        (tag, _, rest) = tlv(rest)?;
    }
    if tag != SEQUENCE {
        return None;
    }
    let (issuer_tag, issuer, rest) = tlv(rest)?;
    (issuer_tag == SEQUENCE).then_some((issuer, rest))
}

/// The issuer name (its DER content) of a CRL.
pub fn issuer_of(crl: &[u8]) -> Option<&[u8]> {
    after_issuer(crl).map(|(issuer, _)| issuer)
}

/// Why a CRL's times cannot be used: each is a refusal at start (the verifier cannot use such a CRL either).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    /// Not well formed up to the times, a time that breaks the rules in the module doc, or an unexpected field after them.
    Unreadable,
    /// No `nextUpdate`: optional in X.509, but rustls-webpki refuses a CRL without one, so the server does too, at start.
    MissingNextUpdate,
    /// A time before 1970-01-01T00:00:00Z, which webpki's time type does not hold.
    BeforeEpoch,
}

/// The `nextUpdate` of a CRL (seconds since the Unix epoch), after checking that its `thisUpdate` is a valid time too and
/// that neither is before 1970 (webpki's floor). Only `revokedCertificates` and the extensions may follow the times.
pub fn next_update_of(crl: &[u8]) -> Result<i64, TimeError> {
    let (_, rest) = after_issuer(crl).ok_or(TimeError::Unreadable)?;
    let (tag, this_update, rest) = tlv(rest).ok_or(TimeError::Unreadable)?;
    checked_time(tag, this_update)?;
    let Some((tag, content, _)) = tlv(rest) else {
        // nothing follows thisUpdate (no revocations, no nextUpdate, no extensions), or junk does
        return Err(if rest.is_empty() {
            TimeError::MissingNextUpdate
        } else {
            TimeError::Unreadable
        });
    };
    match tag {
        UTC_TIME | GENERALIZED_TIME => checked_time(tag, content),
        SEQUENCE | EXTENSIONS => Err(TimeError::MissingNextUpdate),
        _ => Err(TimeError::Unreadable),
    }
}

fn checked_time(tag: u8, content: &[u8]) -> Result<i64, TimeError> {
    match time_to_unix(tag, content) {
        None => Err(TimeError::Unreadable),
        Some(t) if t < 0 => Err(TimeError::BeforeEpoch),
        Some(t) => Ok(t),
    }
}

/// The value of a `UTCTime` (tag 0x17) or `GeneralizedTime` (tag 0x18) in seconds since the Unix epoch (negative before
/// 1970), under the strict rules of the module doc; `None` for anything else.
pub fn time_to_unix(tag: u8, content: &[u8]) -> Option<i64> {
    let (year_digits, rest) = match (tag, content.len()) {
        (UTC_TIME, 13) => (2, content),
        (GENERALIZED_TIME, 15) => (4, content),
        _ => return None,
    };
    let (&last, digits) = rest.split_last()?;
    if last != b'Z' || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let num = |from: usize, len: usize| -> i64 {
        digits[from..from + len]
            .iter()
            .fold(0i64, |a, b| a * 10 + i64::from(b - b'0'))
    };
    let yy = num(0, year_digits);
    let year = match (tag, yy) {
        (UTC_TIME, 50..) => 1900 + yy,
        (UTC_TIME, _) => 2000 + yy,
        _ => yy,
    };
    let at = year_digits;
    let (month, day, hour, minute, second) = (
        num(at, 2),
        num(at + 2, 2),
        num(at + 4, 2),
        num(at + 6, 2),
        num(at + 8, 2),
    );
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's `days_from_civil`); month and day are valid.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
