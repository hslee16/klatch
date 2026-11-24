//! Each client library exposes every method of `proto/wire/methods.json` with the list's params (#1069): the Python
//! client, the TypeScript client and the Rust `klatch::client`. For each method and client the check finds the wrapper
//! the list names and requires that it calls the method by its wire name, exposes every listed param (under the name
//! the list gives for that client, camel-cased in TypeScript) and sends it under its wire key, and exposes no param the
//! list does not have. A gap a client has on purpose is recorded in the list (`clients.<client>.gaps`) and the check
//! fails when the gap is no longer one, so a gap cannot outlive its fix. `client_methods.rs` checks the same wrappers
//! against the PROTOCOL.md tables; the two read different sources and agree through the list. Each check is a
//! function of source text so a negative test can hand it a client that drifted.

mod wire_list;
use serde_json::Value;
use wire_list::{list, methods, name_of, params_of, repo};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Client {
    Python,
    TypeScript,
    Rust,
}

impl Client {
    fn key(self) -> &'static str {
        match self {
            Client::Python => "python",
            Client::TypeScript => "typescript",
            Client::Rust => "rust",
        }
    }
}

/// The sources of a client: its wrapper file, and for Rust the request structs the wrappers take.
struct Source {
    wrappers: String,
    requests: String,
}

fn real(client: Client) -> Source {
    let (file, requests) = match client {
        Client::Python => ("clients/python/klatch_client.py", None),
        Client::TypeScript => ("clients/typescript/klatch.ts", None),
        Client::Rust => (
            "proto/klatch/src/client/methods.rs",
            Some("proto/klatch/src/client/requests.rs"),
        ),
    };
    Source {
        wrappers: repo(file),
        requests: requests.map(repo).unwrap_or_default(),
    }
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

/// A wrapper found in a source: the names its signature exposes and its body text.
struct Wrapper {
    exposed: Vec<String>,
    body: String,
}

/// The text between the `(` at `open` and its matching `)`, and what follows it.
fn parens(text: &str, open: usize) -> (&str, &str) {
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (&text[open + 1..open + i], &text[open + i + 1..]);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses in a wrapper signature")
}

/// Split at the commas that are not inside brackets, braces, angle brackets or parentheses.
fn split_top(text: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut from) = (Vec::new(), 0i32, 0);
    let mut prev = ' ';
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if prev != '=' && prev != '-' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[from..i]);
                from = i + 1;
            }
            _ => {}
        }
        prev = c;
    }
    parts.push(&text[from..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

fn ident(text: &str) -> String {
    text.trim_start_matches("pub ")
        .trim_start_matches("mut ")
        .trim_start_matches("...")
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// The names a TypeScript argument exposes: its own, and the keys of an object type (`opts: { last?: number }`).
fn ts_arg_names(arg: &str) -> Vec<String> {
    // an object type is the options bag: its keys are the params, the bag's own name is not
    if let Some(inner) = arg.split_once(": {").and_then(|(_, r)| r.split_once('}')) {
        return split_semis(inner.0).iter().map(|k| ident(k)).collect();
    }
    vec![ident(arg)]
}

fn split_semis(text: &str) -> Vec<&str> {
    text.split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// The fields of `pub struct name<..> { pub field: T, ... }` in the Rust request structs.
fn rust_fields(requests: &str, ty: &str) -> Vec<String> {
    let Some(at) = requests
        .find(&format!("pub struct {ty}<"))
        .or_else(|| requests.find(&format!("pub struct {ty} ")))
    else {
        return Vec::new();
    };
    requests[at..]
        .lines()
        .skip(1)
        .take_while(|l| l.trim() != "}")
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .map(ident)
        .collect()
}

/// The wrapper `name` of `client` in `src`, or none.
fn find_wrapper(client: Client, src: &Source, name: &str) -> Option<Wrapper> {
    // the wrappers are the methods of the `Client` class; a transport class has methods of the same names
    let class = match client {
        Client::Python => "\nclass Client:",
        Client::TypeScript => "\nexport class Client ",
        Client::Rust => "\nimpl Client {",
    };
    let text = &src.wrappers[src.wrappers.find(class).unwrap_or(0)..];
    let (starts, end_marks): (String, &[&str]) = match client {
        Client::Python => (format!("\n    def {name}("), &["\n    def ", "\n    @"]),
        Client::TypeScript => (
            format!("\n  async {name}("),
            &["\n  async ", "\n  /**", "\n  private ", "\n  static "],
        ),
        Client::Rust => (
            format!("\n    pub fn {name}("),
            &["\n    pub fn ", "\n    /// "],
        ),
    };
    let at = text.find(&starts)? + starts.len() - 1;
    let (sig, rest) = parens(text, at);
    let end = end_marks
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    let mut body = rest[..end].to_string();
    if client == Client::Python {
        // a docstring talks about params (`muted=False`) without sending them
        while let Some((before, after)) = body.split_once("\"\"\"") {
            let tail = after.split_once("\"\"\"").map_or("", |(_, t)| t);
            body = format!("{before}{tail}");
        }
    }
    let mut exposed = Vec::new();
    for arg in split_top(sig) {
        match client {
            Client::Python => {
                if !arg.starts_with('*') && ident(arg) != "self" {
                    exposed.push(ident(arg));
                }
            }
            Client::TypeScript => exposed.extend(ts_arg_names(arg)),
            Client::Rust => {
                let n = ident(arg);
                let ty = arg.split_once(": ").map(|(_, t)| t).unwrap_or("");
                let ty_name = ty.trim_start_matches('&').trim_start_matches("mut ");
                let fields = rust_fields(&src.requests, &ident(ty_name));
                if fields.is_empty() {
                    exposed.push(n);
                    // a typed enum carries params of its own (`Verdict`); the list names it as an alias
                    if ty_name.chars().next().is_some_and(char::is_uppercase)
                        && ty_name.chars().all(char::is_alphanumeric)
                    {
                        exposed.push(ty_name.to_lowercase());
                    }
                } else {
                    exposed.extend(fields);
                }
            }
        }
    }
    exposed.retain(|e| !e.is_empty() && e != "self");
    Some(Wrapper { exposed, body })
}

/// Whether the wrapper body sends the wire key `key`.
fn sends(client: Client, w: &Wrapper, key: &str, exposed_as: &str) -> bool {
    let b = &w.body;
    let quoted = b.contains(&format!("\"{key}\""));
    match client {
        Client::Python => quoted || b.contains(&format!("{key}=")),
        Client::TypeScript => {
            let spread = b.contains("Object.entries(opts)") || b.contains("...opts");
            let shorthand = [
                format!("{{ {key},"),
                format!("{{ {key} }}"),
                format!(", {key} }}"),
                format!(", {key},"),
                format!("{key}:"),
                format!(".{key} = "),
            ]
            .iter()
            .any(|p| b.contains(p.as_str()));
            quoted || shorthand || (spread && w.exposed.iter().any(|e| e == exposed_as))
        }
        Client::Rust => quoted || b.contains(&format!("p.{key}")),
    }
}

fn notes<'a>(m: &'a Value, client: Client) -> &'a Value {
    &m["clients"][client.key()]
}

/// What differs between `client`'s wrappers in `src` and the list; empty when they agree.
fn client_problems(client: Client, src: &Source, list: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    for m in methods(list) {
        let name = name_of(m);
        let wrapper_name = m["wrappers"][client.key()].as_str().expect("wrapper name");
        let Some(w) = find_wrapper(client, src, wrapper_name) else {
            problems.push(format!(
                "{name}: {} client has no wrapper {wrapper_name}",
                client.key()
            ));
            continue;
        };
        if !w.body.contains(&format!("\"{name}\"")) {
            problems.push(format!(
                "{name}: wrapper {wrapper_name} does not call the wire method \"{name}\""
            ));
        }
        let open = m["open_params"].as_bool().unwrap_or(false);
        let note = notes(m, client);
        let alias = |p: &str| note["aliases"][p].as_str().unwrap_or(p).to_string();
        let gaps: Vec<&str> = note["gaps"]
            .as_array()
            .map(|g| g.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let norm = |s: &str| {
            if client == Client::TypeScript {
                camel(s)
            } else {
                s.to_string()
            }
        };
        let params = params_of(m);
        let mut allowed: Vec<String> = Vec::new();
        for (p, _, required) in &params {
            let shown = norm(&alias(p));
            allowed.push(shown.clone());
            let exposed = w.exposed.iter().any(|e| *e == shown);
            if gaps.contains(&p.as_str()) {
                if exposed {
                    problems.push(format!(
                        "{name}: {} wrapper now exposes {p}; remove the gap from the list",
                        client.key()
                    ));
                }
            } else if open && !*required {
                // an open act passes its further params through, so none is named in the signature
            } else if !exposed {
                problems.push(format!(
                    "{name}: {} wrapper {wrapper_name} does not expose param {p}",
                    client.key()
                ));
            } else if !sends(client, &w, p, &shown) {
                problems.push(format!(
                    "{name}: {} wrapper {wrapper_name} does not send param {p} under that key",
                    client.key()
                ));
            }
        }
        for e in &w.exposed {
            let open_bag = open && e == "params";
            if !allowed.contains(e) && !open_bag {
                problems.push(format!("{name}: {} wrapper {wrapper_name} exposes {e}, which is not a param of the method", client.key()));
            }
        }
    }
    problems
}

fn assert_agrees(client: Client) {
    let problems = client_problems(client, &real(client), &list());
    assert!(
        problems.is_empty(),
        "{} client drifted from proto/wire/methods.json:\n{}",
        client.key(),
        problems.join("\n")
    );
}

#[test]
fn the_python_client_matches_the_list() {
    assert_agrees(Client::Python);
}

#[test]
fn the_typescript_client_matches_the_list() {
    assert_agrees(Client::TypeScript);
}

#[test]
fn the_rust_client_matches_the_list() {
    assert_agrees(Client::Rust);
}

/// A client source with `from` replaced by `to` once; panics when the real source no longer has `from`, so a
/// mutation that stops matching is noticed.
fn mutated(client: Client, from: &str, to: &str) -> Source {
    let mut s = real(client);
    assert!(
        s.wrappers.contains(from),
        "the {} source no longer has {from:?}; update this test",
        client.key()
    );
    s.wrappers = s.wrappers.replacen(from, to, 1);
    s
}

fn assert_fails(client: Client, src: &Source, needle: &str) {
    let problems = client_problems(client, src, &list());
    assert!(
        problems.iter().any(|p| p.contains(needle)),
        "expected a problem with {needle:?}, got {problems:?}"
    );
}

#[test]
fn a_method_missing_from_a_client_fails() {
    assert_fails(
        Client::Python,
        &mutated(Client::Python, "def mute(", "def muted("),
        "mute: python client has no wrapper mute",
    );
    assert_fails(
        Client::TypeScript,
        &mutated(Client::TypeScript, "async mute(", "async muted("),
        "mute: typescript client has no wrapper mute",
    );
    assert_fails(
        Client::Rust,
        &mutated(Client::Rust, "pub fn mute(", "pub fn muted("),
        "mute: rust client has no wrapper mute",
    );
}

#[test]
fn a_wrapper_that_calls_another_method_fails() {
    assert_fails(
        Client::Python,
        &mutated(
            Client::Python,
            "self.request(\"mute\"",
            "self.request(\"mutes\"",
        ),
        "does not call the wire method \"mute\"",
    );
    assert_fails(
        Client::TypeScript,
        &mutated(
            Client::TypeScript,
            "this.request(\"mute\"",
            "this.request(\"mutes\"",
        ),
        "does not call the wire method \"mute\"",
    );
    assert_fails(
        Client::Rust,
        &mutated(
            Client::Rust,
            "self.request(\"mute\"",
            "self.request(\"mutes\"",
        ),
        "does not call the wire method \"mute\"",
    );
}

#[test]
fn an_extra_param_in_a_client_fails() {
    assert_fails(
        Client::Python,
        &mutated(
            Client::Python,
            "def mute(self, room,",
            "def mute(self, room, bogus,",
        ),
        "mute: python wrapper mute exposes bogus",
    );
    assert_fails(
        Client::TypeScript,
        &mutated(
            Client::TypeScript,
            "async mute(room: string,",
            "async mute(room: string, bogus: number,",
        ),
        "mute: typescript wrapper mute exposes bogus",
    );
    assert_fails(
        Client::Rust,
        &mutated(
            Client::Rust,
            "pub fn mute(&mut self, room: &str,",
            "pub fn mute(&mut self, room: &str, bogus: u8,",
        ),
        "mute: rust wrapper mute exposes bogus",
    );
}

#[test]
fn a_param_a_client_does_not_expose_fails() {
    assert_fails(
        Client::Python,
        &mutated(
            Client::Python,
            "def mute(self, room, muted=True)",
            "def mute(self, room)",
        ),
        "does not expose param muted",
    );
    assert_fails(
        Client::TypeScript,
        &mutated(
            Client::TypeScript,
            "async mute(room: string, muted = true)",
            "async mute(room: string)",
        ),
        "does not expose param muted",
    );
    assert_fails(
        Client::Rust,
        &mutated(
            Client::Rust,
            "pub fn mute(&mut self, room: &str, muted: bool)",
            "pub fn mute(&mut self, room: &str)",
        ),
        "does not expose param muted",
    );
}

#[test]
fn a_param_sent_under_another_key_fails() {
    assert_fails(
        Client::Python,
        &mutated(Client::Python, "muted=bool(muted)", "mute=bool(muted)"),
        "does not send param muted",
    );
    assert_fails(
        Client::TypeScript,
        &mutated(
            Client::TypeScript,
            "{ room, muted }",
            "{ room, mutedd: muted }",
        ),
        "does not send param muted",
    );
    assert_fails(
        Client::Rust,
        &mutated(Client::Rust, "\"muted\": muted", "\"mutedd\": muted"),
        "does not send param muted",
    );
}

#[test]
fn a_recorded_gap_that_was_closed_fails() {
    // claim's `shared` is a recorded Rust gap; a Rust claim that sent it must have the gap removed from the list
    let mut s = real(Client::Rust);
    s.requests = s.requests.replacen(
        "pub reason: Option<&'a str>,",
        "pub reason: Option<&'a str>,\n    pub shared: bool,",
        1,
    );
    assert_fails(Client::Rust, &s, "claim: rust wrapper now exposes shared");
}
