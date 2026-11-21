//! `SharedTransport` must name every `Transport` method (#1092).
//!
//! A required trait method with no forward already fails to compile (E0046), but only once both changes are in the
//! tree, and a method with a default body is skipped without a word: the handle would run the default on the caller's
//! thread instead of the owner's. This reads both sources and fails with the names of the methods that have no forward.

const TRANSPORT: &str = include_str!("../src/gateway/transport.rs");
const SHARED: &str = include_str!("../src/gateway/shared.rs");

/// Trait methods the handle deliberately leaves to the trait's own default, because the default only calls forwarded
/// methods. Add a name here only with that reason.
const DEFAULTED_ON_PURPOSE: &[&str] = &["post"];

/// The text between `start` and the first line that is a lone closing brace after it.
fn block<'a>(src: &'a str, start: &str) -> &'a str {
    let from = src
        .find(start)
        .unwrap_or_else(|| panic!("`{start}` not found"));
    let rest = &src[from..];
    let end = rest.find("\n}\n").expect("block end");
    &rest[..end]
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

#[test]
fn shared_transport_forwards_every_transport_method() {
    let trait_methods = fn_names(block(TRANSPORT, "pub trait Transport {"), 4);
    let imp = block(SHARED, "impl Transport for SharedTransport {");
    let mut named = fn_names(imp, 4);
    // Entries of the `forward! { ... }` list: `name(args) -> ret;` at eight spaces.
    for l in imp.lines() {
        if let Some(rest) = l.strip_prefix("        ")
            && rest.ends_with(';')
            && !rest.starts_with(' ')
            && let Some(p) = rest.find('(')
        {
            named.push(rest[..p].to_string());
        }
    }
    assert!(
        trait_methods.len() > 50 && named.len() > 50,
        "the parse found {} trait methods and {} forwards; the layout of the sources changed",
        trait_methods.len(),
        named.len()
    );
    let missing: Vec<&String> = trait_methods
        .iter()
        .filter(|m| !named.contains(m) && !DEFAULTED_ON_PURPOSE.contains(&m.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "SharedTransport (proto/klatch/src/gateway/shared.rs) has no forward for these Transport methods: {missing:?}. \
         Add each to the `forward!` list (or write it out if it stores a message), or list it in DEFAULTED_ON_PURPOSE \
         with the reason."
    );
}
