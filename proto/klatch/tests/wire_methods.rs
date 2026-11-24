//! The one list of the wire methods, `proto/wire/methods.json` (#1069), against what the server accepts and what
//! PROTOCOL.md documents. A method added to the session loop and not to the list fails here, as does a listed method
//! the loop does not accept, a param that differs (name, whether it is required, its type), and a method or param
//! PROTOCOL.md section 5 does not show. The three client libraries are checked against the same list in
//! `client_params.rs`. Each check is a function of source text, so a negative test feeds it a source that drifted and
//! sees it fail.

mod wire_list;
use serde_json::Value;
use std::collections::BTreeMap;
use wire_list::{list, methods, name_of, params_of, repo, with_edit, without_method};

/// The method names of the dispatch arms (`"post" => ...`, `"a" | "b" => ...`) in a source file.
fn arms(src: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in src.lines() {
        let Some((head, _)) = line.trim().split_once(" =>") else {
            continue;
        };
        let names: Vec<&str> = head.trim_start_matches("| ").split(" | ").collect();
        let is_name = |n: &&str| {
            n.len() > 2
                && n.starts_with('"')
                && n.ends_with('"')
                && n[1..n.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
        };
        if names.iter().all(is_name) {
            found.extend(names.iter().map(|n| n[1..n.len() - 1].to_string()));
        }
    }
    found
}

/// One field of a params struct, as `(name, base type, required)`.
type Field = (String, String, bool);

/// The fields of `struct name { ... }` found in any of `sources`: required when it has no `#[serde(default)]` and is
/// not an `Option`; the type is the base word (`string`, `integer`, `boolean`, `array`).
fn struct_fields(sources: &[&str], name: &str) -> Vec<Field> {
    let head = format!("struct {name} {{");
    let body = sources
        .iter()
        .find_map(|s| s.split(&head).nth(1))
        .unwrap_or_else(|| panic!("no struct {name} in the session sources"));
    let (mut fields, mut default) = (Vec::new(), false);
    for line in body.lines().map(str::trim).take_while(|l| *l != "}") {
        if line.starts_with("#[serde(default") {
            default = true;
        } else if line.starts_with("#[serde(") {
            panic!(
                "{name}: `{line}`: a rename, alias or flatten changes the wire names; teach struct_fields in tests/wire_methods.rs"
            );
        } else if let Some((n, ty)) = line.split_once(": ").filter(|_| !line.starts_with("//")) {
            let ty = ty.trim_end_matches(',');
            let optional = ty.starts_with("Option<");
            let base = ty.trim_start_matches("Option<").trim_end_matches('>');
            fields.push((
                n.trim_start_matches("pub ").to_string(),
                json_type(base),
                !default && !optional,
            ));
            default = false;
        } else if !line.starts_with("///") && !line.starts_with("//") {
            default = false;
        }
    }
    fields
}

fn json_type(rust: &str) -> String {
    match rust {
        "String" => "string",
        "bool" => "boolean",
        "i64" | "u64" | "usize" | "u32" => "integer",
        t if t.starts_with("Vec<") => "array",
        other => panic!("a param of Rust type {other}: teach json_type in tests/wire_methods.rs"),
    }
    .to_string()
}

/// Methods whose arm does not name its params struct in a `|p: Type|` closure, with the struct they read (`None`: no
/// params). A new method of this shape must be added here, and the test says so when it is not.
const IRREGULAR: &[(&str, Option<&str>)] = &[
    ("prune", None),
    ("review_next", None),
    ("role_act", Some("ActParams")),
    ("review_verdict", Some("VerdictParams")),
    ("subscribe", Some("SubscribeParams")),
    ("unsubscribe", Some("RoomParams")),
];

/// Every method the session loop accepts with its params, read from the three source files of the dispatch.
fn dispatch(methods_rs: &str, role_rs: &str, mod_rs: &str) -> BTreeMap<String, Vec<Field>> {
    let sources = [methods_rs, role_rs, mod_rs];
    let mut names = arms(methods_rs);
    names.extend(arms(mod_rs));
    names.sort();
    names.dedup();
    let mut out = BTreeMap::new();
    for name in names {
        let ty = match IRREGULAR.iter().find(|(n, _)| *n == name) {
            Some((_, ty)) => ty.map(str::to_string),
            None => Some(closure_type(methods_rs, &name).unwrap_or_else(|| {
                panic!("method {name}: no `|p: Type|` params closure in its arm; add it to IRREGULAR in tests/wire_methods.rs")
            })),
        };
        let fields = match ty.as_deref() {
            None | Some("NoParams") => Vec::new(),
            Some(t) => struct_fields(&sources, t),
        };
        out.insert(name, fields);
    }
    out
}

/// The params type of the arm for `name` in `src`: the `Type` of the first `|p: Type|` after `"name" => call.run(`.
fn closure_type(src: &str, name: &str) -> Option<String> {
    let at = src.find(&format!("\"{name}\" => call.run("))?;
    let rest = &src[at..];
    let bar = rest.find("|")?;
    let closure = &rest[bar + 1..];
    let (arg, _) = closure.split_once('|')?;
    arg.split_once(": ").map(|(_, t)| t.trim().to_string())
}

/// What differs between the list and the dispatch; empty when they agree.
fn list_problems(list: &Value, dispatch: &BTreeMap<String, Vec<Field>>) -> Vec<String> {
    let mut problems = Vec::new();
    for m in methods(list) {
        let name = name_of(m);
        let Some(accepted) = dispatch.get(name) else {
            problems.push(format!(
                "{name} is in the list but the session loop does not accept it"
            ));
            continue;
        };
        let listed = params_of(m);
        for (n, ty, req) in &listed {
            match accepted.iter().find(|(an, _, _)| an == n) {
                None => problems.push(format!("{name}: listed param {n} is not read by the server")),
                Some((_, aty, areq)) if aty != ty || areq != req => problems.push(format!(
                    "{name}: param {n} is {ty} required={req} in the list but {aty} required={areq} on the server"
                )),
                Some(_) => {}
            }
        }
        for (n, _, _) in accepted {
            if !listed.iter().any(|(ln, _, _)| ln == n) {
                problems.push(format!(
                    "{name}: the server reads param {n}, the list does not have it"
                ));
            }
        }
    }
    for name in dispatch.keys() {
        if !methods(list).iter().any(|m| name_of(m) == name) {
            problems.push(format!(
                "{name} is accepted by the session loop but is not in the list"
            ));
        }
    }
    problems
}

fn real_dispatch() -> BTreeMap<String, Vec<Field>> {
    dispatch(
        &repo("proto/klatch/src/session/methods.rs"),
        &repo("proto/klatch/src/session/role_methods.rs"),
        &repo("proto/klatch/src/session/mod.rs"),
    )
}

#[test]
fn the_parsers_find_what_they_should() {
    let d = real_dispatch();
    for known in [
        "post",
        "claim",
        "role_act",
        "subscribe",
        "unsubscribe",
        "prune",
        "kind",
    ] {
        assert!(d.contains_key(known), "{known} not in {:?}", d.keys());
    }
    let post: Vec<&str> = d["post"].iter().map(|f| f.0.as_str()).collect();
    assert_eq!(
        post,
        ["room", "body", "label", "kind", "payload", "scope", "refs"]
    );
    assert!(
        d["post"][0].2 && !d["post"][2].2,
        "room required, label optional"
    );
    assert!(d["my_rooms"].is_empty(), "no params");
}

#[test]
fn the_list_and_the_session_loop_agree_on_every_method_and_param() {
    let problems = list_problems(&list(), &real_dispatch());
    assert!(
        problems.is_empty(),
        "proto/wire/methods.json drifted from the wire:\n{}",
        problems.join("\n")
    );
}

#[test]
fn a_method_only_the_server_accepts_fails_the_list_check() {
    let (m, r, s) = (
        repo("proto/klatch/src/session/methods.rs"),
        repo("proto/klatch/src/session/role_methods.rs"),
        repo("proto/klatch/src/session/mod.rs"),
    );
    // one more arm and its params struct, as a new wire method would add
    let m = m.replace(
        "\"my_rooms\" => call.run(",
        "\"fresh\" => call.run(|p: RoomParams| g.who(who, &p.room), |ms| members_json(&ms)),\n        \"my_rooms\" => call.run(",
    );
    let problems = list_problems(&list(), &dispatch(&m, &r, &s));
    assert!(
        problems.iter().any(|p| p.starts_with("fresh is accepted")),
        "{problems:?}"
    );
}

#[test]
fn a_method_only_the_list_has_fails_the_list_check() {
    let mut l = list();
    l["methods"]
        .as_array_mut()
        .expect("methods")
        .push(serde_json::json!(
            {"name": "ghost", "mode": "read", "room_arg": null, "params": [], "result": "{}"}
        ));
    let problems = list_problems(&l, &real_dispatch());
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("ghost is in the list")),
        "{problems:?}"
    );
    let problems = list_problems(&without_method(&list(), "post"), &real_dispatch());
    assert!(
        problems.iter().any(|p| p.starts_with("post is accepted")),
        "{problems:?}"
    );
}

#[test]
fn a_param_that_differs_fails_the_list_check() {
    let dispatch = real_dispatch();
    // required flipped, type changed, param dropped, param invented
    let flipped = with_edit(&list(), "post", |m| {
        m["params"][2]["required"] = true.into()
    });
    let retyped = with_edit(&list(), "read", |m| {
        m["params"][1]["type"] = "string".into()
    });
    let dropped = with_edit(&list(), "claim", |m| {
        m["params"].as_array_mut().expect("params").pop();
    });
    let invented = with_edit(&list(), "who", |m| {
        m["params"]
            .as_array_mut()
            .expect("params")
            .push(serde_json::json!(
                {"name": "extra", "type": "string", "required": false}
            ));
    });
    for (l, needle) in [
        (flipped, "post: param label is"),
        (retyped, "read: param peek is"),
        (dropped, "claim: the server reads param shared"),
        (invented, "who: listed param extra is not read"),
    ] {
        let problems = list_problems(&l, &dispatch);
        assert!(
            problems.iter().any(|p| p.starts_with(needle)),
            "{needle}: {problems:?}"
        );
    }
}

/// The rows of PROTOCOL.md section 5 as `(method, params cell)`.
fn spec_rows(spec: &str) -> Vec<(String, String)> {
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
        .filter_map(|l| {
            let (name, rest) = l.split_once('`')?;
            let params = rest.split('|').nth(1)?.trim().to_string();
            let plain = name.chars().all(|c| c.is_ascii_lowercase() || c == '_');
            (plain && !name.is_empty()).then(|| (name.to_string(), params))
        })
        .collect()
}

/// What differs between the list and the PROTOCOL.md tables: a method missing on either side, a listed param the
/// row's Params cell does not show in backticks.
fn spec_problems(list: &Value, rows: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for m in methods(list) {
        let name = name_of(m);
        let Some((_, cell)) = rows.iter().find(|(n, _)| n == name) else {
            problems.push(format!(
                "{name} is in the list but PROTOCOL.md section 5 has no row for it"
            ));
            continue;
        };
        for (p, _, _) in params_of(m) {
            if !cell.contains(&format!("`{p}`")) {
                problems.push(format!(
                    "{name}: PROTOCOL.md does not show param {p} in its row"
                ));
            }
        }
    }
    for (n, _) in rows {
        if !methods(list).iter().any(|m| name_of(m) == n) {
            problems.push(format!("{n} has a PROTOCOL.md row but is not in the list"));
        }
    }
    problems
}

#[test]
fn protocol_md_and_the_list_show_the_same_methods_and_params() {
    let problems = spec_problems(&list(), &spec_rows(&repo("PROTOCOL.md")));
    assert!(
        problems.is_empty(),
        "PROTOCOL.md drifted from the list:\n{}",
        problems.join("\n")
    );
}

#[test]
fn a_method_or_param_missing_from_protocol_md_fails_the_spec_check() {
    let spec = repo("PROTOCOL.md");
    let rows = spec_rows(&spec);
    let gone: Vec<_> = rows.iter().filter(|(n, _)| n != "mute").cloned().collect();
    let problems = spec_problems(&list(), &gone);
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("mute is in the list")),
        "{problems:?}"
    );
    let extra: Vec<_> = rows
        .iter()
        .cloned()
        .chain([("ghost".to_string(), String::new())])
        .collect();
    assert!(
        spec_problems(&list(), &extra)
            .iter()
            .any(|p| p.starts_with("ghost has a"))
    );
    let thin: Vec<_> = rows
        .iter()
        .map(|(n, c)| {
            (
                n.clone(),
                if n == "mute" {
                    c.replace("`muted`", "muted")
                } else {
                    c.clone()
                },
            )
        })
        .collect();
    let problems = spec_problems(&list(), &thin);
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("mute: PROTOCOL.md does not show param muted")),
        "{problems:?}"
    );
}

#[test]
fn the_list_is_well_formed() {
    let l = list();
    assert_eq!(l["schema"], 1);
    let names: Vec<&str> = methods(&l).iter().map(name_of).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted, "methods are sorted by name and unique");
    for m in methods(&l) {
        let name = name_of(m);
        assert!(
            ["read", "act"].contains(&m["mode"].as_str().unwrap_or("")),
            "{name}: mode is read or act"
        );
        assert!(
            m["result"].as_str().is_some_and(|r| !r.is_empty()),
            "{name}: result shape"
        );
        let params = params_of(m);
        for (p, ty, _) in &params {
            assert!(
                ["string", "integer", "boolean", "array"].contains(&ty.as_str()),
                "{name}.{p}: type {ty}"
            );
        }
        match m["room_arg"].as_str() {
            Some(a) => assert!(
                params.iter().any(|(p, _, _)| p == a),
                "{name}: room_arg {a} is not a param"
            ),
            None => assert!(
                m["room_arg"].is_null(),
                "{name}: room_arg is a param name or null"
            ),
        }
        let w = m["wrappers"].as_object().expect("wrappers");
        assert!(
            ["python", "typescript", "rust"]
                .iter()
                .all(|c| w.contains_key(*c)),
            "{name}: a wrapper per client"
        );
    }
}
