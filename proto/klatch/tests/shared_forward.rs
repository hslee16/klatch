//! `SharedTransport` must name every `Transport` method (#1092, #1071).
//!
//! A required trait method with no forward already fails to compile (E0046), but only once both changes are in the
//! tree, and a method with a default body is skipped without a word: the handle would run the default on the caller's
//! thread instead of the owner's. This walks `src/gateway/transport/`, where each area file holds a trait and the
//! `impl <Trait> for SharedTransport` block that forwards it, and fails with the names of the methods that have none.

use std::fs;
use std::path::PathBuf;

/// Trait methods the handle deliberately leaves to the trait's own default, because the default only calls forwarded
/// methods. Add a name here only with that reason.
const DEFAULTED_ON_PURPOSE: &[&str] = &["post"];

/// The text from the line `start` up to the first line that is a lone closing brace after it.
fn block<'a>(src: &'a str, start: &str) -> Option<&'a str> {
    let from = src.find(start)?;
    let rest = &src[from..];
    let end = rest.find("\n}\n").expect("block end");
    Some(&rest[..end])
}

/// The name after `fn ` on lines indented by exactly `indent` spaces.
fn fn_names(text: &str, indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    text.lines()
        .filter_map(|l| l.strip_prefix(&format!("{pad}fn ")))
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        })
        .collect()
}

/// Names the handle's impl block of one area trait carries: hand-written methods and `forward! { ... }` entries
/// (`name(args) -> ret;` at eight spaces).
fn forwarded(imp: &str) -> Vec<String> {
    let mut named = fn_names(imp, 4);
    for l in imp.lines() {
        if let Some(rest) = l.strip_prefix("        ")
            && rest.ends_with(';')
            && !rest.starts_with(' ')
            && let Some(p) = rest.find('(')
        {
            named.push(rest[..p].to_string());
        }
    }
    named
}

#[test]
fn shared_transport_forwards_every_transport_method() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/gateway/transport");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    let (mut traits, mut total, mut missing) = (0, 0, Vec::new());
    for file in &files {
        let src = fs::read_to_string(file).unwrap();
        let names: Vec<String> = src
            .lines()
            .filter_map(|l| l.strip_prefix("pub trait "))
            .filter_map(|l| l.strip_suffix(" {"))
            .map(str::to_string)
            .collect();
        for name in names {
            traits += 1;
            let methods = fn_names(block(&src, &format!("pub trait {name} {{")).unwrap(), 4);
            let imp =
                block(&src, &format!("impl {name} for SharedTransport {{")).unwrap_or_else(|| {
                    panic!("{}: no `impl {name} for SharedTransport`", file.display())
                });
            let named = forwarded(imp);
            total += methods.len();
            for m in methods {
                if !named.contains(&m) && !DEFAULTED_ON_PURPOSE.contains(&m.as_str()) {
                    missing.push(format!("{name}::{m}"));
                }
            }
        }
    }
    assert!(
        traits >= 8 && total > 50,
        "the parse found {traits} area traits and {total} methods; the layout of the sources changed"
    );
    assert!(
        missing.is_empty(),
        "SharedTransport (proto/klatch/src/gateway/shared.rs and the area files) has no forward for these Transport \
         methods: {missing:?}. Add each to the `forward!` list of its area's `impl .. for SharedTransport` block (or \
         write it out if it stores a message), or list it in DEFAULTED_ON_PURPOSE with the reason."
    );
}
