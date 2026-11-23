//! Every method PROTOCOL.md lists has a wrapper in each of the three client libraries (#747): the Python client, the
//! TypeScript client and the Rust `klatch::client`. The codec test already fails when the spec misses a method the
//! gateway answers; this one fails when a method in the spec has no wrapper, so a new method cannot ship with the
//! clients left behind. A wrapper is named for the method (`post`), or `set_` plus the method for one that declares
//! something about the caller (`set_scope`, `set_kind`, `set_status`).

use std::fs;
use std::path::PathBuf;

fn repo(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The method names in the tables of PROTOCOL.md section 5 (the first backticked cell of each row).
fn spec_methods() -> Vec<String> {
    let spec = repo("PROTOCOL.md");
    let section = spec
        .split("## 5. Methods")
        .nth(1)
        .expect("section 5")
        .split("### Error codes")
        .next()
        .expect("error codes");
    section
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .filter(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_') && !n.is_empty())
        .map(str::to_string)
        .collect()
}

fn camel(snake: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn defines(source: &str, starts: &[String]) -> bool {
    source
        .lines()
        .map(str::trim_start)
        .any(|l| starts.iter().any(|s| l.starts_with(s.as_str())))
}

#[test]
fn the_spec_lists_the_methods_this_check_expects() {
    let methods = spec_methods();
    for known in [
        "post",
        "read",
        "role_act",
        "review_verdict",
        "invite",
        "kind",
    ] {
        assert!(methods.iter().any(|m| m == known), "{known} in {methods:?}");
    }
}

/// Every spec method has a wrapper in `source` under one of the names `names_of` gives for it.
fn assert_every_method_wrapped(source: &str, client: &str, names_of: &dyn Fn(&str) -> [String; 2]) {
    let source = repo(source);
    for m in spec_methods() {
        assert!(
            defines(&source, &names_of(&m)),
            "{client} client has no wrapper for {m}"
        );
    }
}

#[test]
fn every_method_in_the_spec_has_a_python_wrapper() {
    assert_every_method_wrapped("clients/python/klatch_client.py", "Python", &|m| {
        [format!("def {m}("), format!("def set_{m}(")]
    });
}

#[test]
fn every_method_in_the_spec_has_a_typescript_wrapper() {
    assert_every_method_wrapped("clients/typescript/klatch.ts", "TypeScript", &|m| {
        let c = camel(m);
        [
            format!("async {c}("),
            format!("async set{}(", capitalise(&c)),
        ]
    });
}

fn capitalise(s: &str) -> String {
    let mut cs = s.chars();
    cs.next()
        .map(|f| f.to_uppercase().collect::<String>() + cs.as_str())
        .unwrap_or_default()
}

#[test]
fn every_method_in_the_spec_has_a_rust_wrapper() {
    assert_every_method_wrapped("proto/klatch/src/client/methods.rs", "Rust", &|m| {
        [format!("pub fn {m}("), format!("pub fn set_{m}(")]
    });
}
