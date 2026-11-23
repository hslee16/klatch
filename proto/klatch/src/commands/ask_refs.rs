//! The refs of `klatch ask` (#1318): `--ref`, `--ref-label` and `--ref-snippet-file` turned into the refs the gateway
//! checks. Nothing here fetches or opens what a ref points at.

use anyhow::{Result, bail};

/// What a dm ask says to `--ref` (#1318): a dm ask carries no refs, as it carries no declared scope (#1322).
pub(super) const DM_REFUSAL: &str =
    "--ref goes with a room ask: klatch ask ROOM TEXT --ref KIND:VALUE (a dm ask carries no refs)";

/// Whether any of the ref flags was given.
pub(super) fn given(refs: &[String], labels: &[String], files: &[String]) -> bool {
    !(refs.is_empty() && labels.is_empty() && files.is_empty())
}

/// The refs of `klatch ask` from `--ref KIND:VALUE`, `--ref-label` and `--ref-snippet-file` (#1318). Labels and snippet
/// files go with the refs in order; an empty one leaves that ref without. A snippet file is read here, by the CLI,
/// and refused whole when it is over the cap or not text; nothing is cut. Nothing here fetches or opens what a ref
/// points at: the value is only text.
pub(super) fn build_refs(
    refs: &[String],
    labels: &[String],
    files: &[String],
) -> Result<Vec<klatch::domain::AskRef>> {
    check_counts(refs, labels, files)?;
    refs.iter()
        .enumerate()
        .map(|(i, raw)| one_ref(i, raw, labels, files))
        .collect()
}

/// Labels and snippet files go with refs: none without a ref, and never more of them than refs.
fn check_counts(refs: &[String], labels: &[String], files: &[String]) -> Result<()> {
    if refs.is_empty() && given(refs, labels, files) {
        bail!("--ref-label and --ref-snippet-file go with --ref KIND:VALUE; give a --ref first");
    }
    if labels.len() > refs.len() || files.len() > refs.len() {
        bail!(
            "there are more --ref-label or --ref-snippet-file than --ref: they go with the --refs in order, and an empty '' leaves one without"
        );
    }
    Ok(())
}

/// The `i`th ref (counted from 0) with its label and snippet file, if any.
fn one_ref(
    i: usize,
    raw: &str,
    labels: &[String],
    files: &[String],
) -> Result<klatch::domain::AskRef> {
    let n = i + 1;
    let Some((kind, value)) = raw.split_once(':') else {
        bail!("--ref {n} is not KIND:VALUE (for example `pr:owner/repo#12`)");
    };
    if !klatch::domain::valid_ref_kind(kind) {
        bail!("--ref {n}: bad kind: 1 to 32 characters of a-z 0-9 _ . - before the first `:`");
    }
    let label = labels.get(i).filter(|l| !l.is_empty()).cloned();
    let snippet = match files.get(i).filter(|f| !f.is_empty()) {
        Some(path) => Some(read_snippet(path)?),
        None => None,
    };
    Ok(klatch::domain::AskRef {
        kind: kind.to_string(),
        value: value.to_string(),
        label,
        snippet,
    })
}

/// The text of a snippet file, refused whole when it is over the cap or is not UTF-8 text.
fn read_snippet(path: &str) -> Result<String> {
    use std::io::Read;
    let cap = klatch::domain::MAX_SNIPPET_BYTES;
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("cannot read snippet file {path}: {e}"))?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("cannot read snippet file {path}: {e}"))?;
    if bytes.len() > cap {
        bail!(
            "the snippet file {path} is over {cap} bytes; nothing was cut, send the relevant part"
        );
    }
    String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("the snippet file {path} is not UTF-8 text"))
}
