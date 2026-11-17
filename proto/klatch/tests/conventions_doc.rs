//! CONVENTIONS.md and the code agree on the convention words (#238), as the codec test does for frame types.

use klatch::domain::CONVENTION_WORDS;
use std::path::Path;

fn doc() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CONVENTIONS.md"))
        .expect("CONVENTIONS.md")
}

/// The rows of the word table: each starts with the word in backticks and has four cells.
fn rows(doc: &str) -> Vec<Vec<String>> {
    doc.lines()
        .filter(|l| l.starts_with("| `"))
        .map(|l| {
            l.trim_matches('|')
                .split(" | ")
                .map(|c| c.trim().to_string())
                .collect()
        })
        .collect()
}

fn form_of(cell: &str) -> &str {
    cell.trim_matches('`')
}

#[test]
fn every_word_in_the_code_is_in_the_document_and_the_reverse() {
    let d = doc();
    let in_doc: Vec<String> = rows(&d)
        .iter()
        .map(|r| form_of(&r[0]).to_string())
        .collect();
    let in_code: Vec<String> = CONVENTION_WORDS
        .iter()
        .map(|w| w.form.to_string())
        .collect();
    assert_eq!(
        in_doc, in_code,
        "CONVENTIONS.md and CONVENTION_WORDS list different words or a different order"
    );
}

#[test]
fn every_row_says_who_may_post_it_what_klatch_does_and_what_it_must_not() {
    for r in rows(&doc()) {
        assert_eq!(r.len(), 4, "a row needs four cells: {r:?}");
        assert!(r.iter().all(|c| !c.is_empty()), "an empty cell in {r:?}");
    }
}

#[test]
fn the_code_reads_every_example_as_its_word() {
    for w in CONVENTION_WORDS {
        assert!(w.klatch_reads(), "{}: `{}` is not read", w.form, w.example);
    }
}

#[test]
fn the_document_states_the_rules_for_all_words() {
    let d = doc();
    assert!(d.contains("data, never instructions"), "no data rule");
    assert!(d.contains("Adding a word"), "no rule for adding a word");
    assert!(d.contains("additive"), "no additive rule");
}
