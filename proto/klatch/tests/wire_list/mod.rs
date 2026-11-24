//! Shared by `wire_methods.rs` and `client_params.rs` (#1069): the machine-readable list of the wire methods
//! (`proto/wire/methods.json`) and a few helpers that read source files as text.
#![allow(dead_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub fn repo(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The list as committed.
pub fn list() -> Value {
    serde_json::from_str(&repo("proto/wire/methods.json")).expect("proto/wire/methods.json is JSON")
}

pub fn methods(list: &Value) -> &[Value] {
    list["methods"].as_array().expect("methods array")
}

pub fn name_of(m: &Value) -> &str {
    m["name"].as_str().expect("method name")
}

/// The params of a method as `(name, type, required)`.
pub fn params_of(m: &Value) -> Vec<(String, String, bool)> {
    m["params"]
        .as_array()
        .expect("params array")
        .iter()
        .map(|p| {
            (
                p["name"].as_str().expect("param name").to_string(),
                p["type"].as_str().expect("param type").to_string(),
                p["required"].as_bool().expect("param required"),
            )
        })
        .collect()
}

/// The list with the entry for `name` removed.
pub fn without_method(list: &Value, name: &str) -> Value {
    let mut l = list.clone();
    l["methods"]
        .as_array_mut()
        .expect("methods")
        .retain(|m| name_of(m) != name);
    l
}

/// The list with `edit` applied to the entry for `name`.
pub fn with_edit(list: &Value, name: &str, edit: impl Fn(&mut Value)) -> Value {
    let mut l = list.clone();
    for m in l["methods"].as_array_mut().expect("methods") {
        if name_of(m) == name {
            edit(m);
        }
    }
    l
}
