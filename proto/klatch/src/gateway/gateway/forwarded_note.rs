//! The server's note of forwarded identity headers it ignored (#1490, epic #13). `klatch-server http` and `https` count
//! requests that carried a forwarded header, by header name only, in memory; a timer (never a request) hands the
//! counts here, and they are kept as one audit row per interval so the count survives a restart. The row is the
//! server's own operational note, like a session-level refusal: it grants nothing and reads nothing.

use super::*;
use crate::store::NewAudit;

/// The action of the row. Fixed; a peer never picks it.
pub(crate) const FORWARDED_IGNORED: &str = crate::store::FORWARDED_IGNORED_ACTION;
/// The longest detail kept: ten names and one prefix family, each with a count, fit in a fraction of this.
const MAX_DETAIL: usize = 512;

/// Whether `text` is only lower case letters, digits and the few marks of `name=count` pairs: the shape of the counts
/// line, so no header value (an address, a user, a certificate) can be kept even by a bug in the caller.
fn names_and_counts_only(text: &str) -> bool {
    text.len() <= MAX_DETAIL
        && text
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'=' | b'-' | b'*' | b' '))
}

impl<S: Store> Gateway<S> {
    /// Keep one `forwarded-ignored` audit row with `counts` (`requests=N name=K ...`). Refused, and nothing kept, when
    /// the text is anything but names and counts.
    pub(crate) fn note_forwarded_ignored(&self, counts: &str) -> Result<()> {
        if !names_and_counts_only(counts) {
            return Err(Error::Invalid(
                "the forwarded-ignored note holds header names and counts only".into(),
            ));
        }
        self.store
            .record_audit(&NewAudit::new(FORWARDED_IGNORED, "", counts), self.now())
    }
}
