//! Error text never repeats a caller's name raw (#1768, after #1763): a name that failed validation is echoed capped
//! and escaped, so a 100 KB name, a control, bidi or tag character or an ANSI sequence cannot steer a terminal or
//! flood a reader through an error.

use super::*;
use crate::gateway::ClaimSpec;
use crate::gateway::CredentialSpec;

/// A name that is long and carries every kind of character an error must not pass on.
fn hostile() -> String {
    format!(
        "{}\u{1b}[31m\u{1b}]0;pwned\u{7}\u{202e}\u{2066}\u{e0041}\u{e0001}\u{200b}\r\n\t\u{0}x\\u{{41}}",
        "n".repeat(100 * 1024)
    )
}

/// The sites checked so far and what each got wrong; every site is tried before the test fails, so one run shows them
/// all.
#[derive(Default)]
struct Sites(Vec<String>);

impl Sites {
    /// The error of `r` must be bounded pure ASCII without a control character.
    fn check(&mut self, r: Result<impl std::fmt::Debug>, site: &str) {
        let Err(e) = r else {
            self.0.push(format!("{site}: no error"));
            return;
        };
        let text = e.to_string();
        let shown: String = text
            .chars()
            .take(60)
            .collect::<String>()
            .escape_debug()
            .to_string();
        if !text.is_ascii() {
            self.0.push(format!(
                "{site}: not pure ASCII ({} bytes): {shown}",
                text.len()
            ));
        } else if text.chars().any(|c| c.is_control()) {
            self.0.push(format!("{site}: a control character: {shown}"));
        } else if text.len() >= 600 {
            self.0
                .push(format!("{site}: {} bytes: {shown}", text.len()));
        }
    }

    fn finish(self) {
        assert!(self.0.is_empty(), "\n{}", self.0.join("\n"));
    }
}

fn hostile_text() -> String {
    hostile()
}

#[test]
fn every_member_name_echo_is_bounded_and_ascii() {
    let g = open();
    let h = hostile_text();
    let (a, r) = (admin(), "r");
    let cases: Vec<(&str, Result<()>)> = vec![
        ("admin_remove_member", g.admin_remove_member(&a, r, &h)),
        ("admin_allow_member", g.admin_allow_member(&a, r, &h)),
        (
            "grant_room_clearance",
            g.grant_room_clearance(&a, r, &h, "internal"),
        ),
        (
            "admin_set_room_grantor",
            g.admin_set_room_grantor(&a, r, &h, true),
        ),
        (
            "admin_set_room_clearance",
            g.admin_set_room_clearance(&a, r, &h, None),
        ),
        (
            "admin_set_clearance",
            g.admin_set_clearance(&a, &h, "internal"),
        ),
        ("admin_set_reviewer", g.admin_set_reviewer(&a, &h, true)),
        (
            "admin_set_admin_agent",
            g.admin_set_admin_agent(&a, &h, true),
        ),
        ("admin_set_ssh_key", g.admin_set_ssh_key(&a, "k", &h, true)),
        ("admin_revoke_credential", g.admin_revoke_credential(&a, &h)),
        (
            "admin_grant_room_capability",
            g.admin_grant_room_capability(&a, &h, true),
        ),
        ("invite member", g.invite(&alice(), &h, r, r)),
        ("invite room", g.invite(&alice(), "bob", &h, r)),
        ("invite via", g.invite(&alice(), "bob", r, &h)),
    ];
    let mut sites = Sites::default();
    for (site, res) in cases {
        sites.check(res, site);
    }
    sites.check(g.could_answer(&a, r, &h, 1), "could_answer");
    sites.finish();
}

#[test]
fn the_other_name_echoes_are_bounded_and_ascii() {
    let g = open();
    let h = hostile_text();
    let a = admin();
    let mut sites = Sites::default();
    sites.check(g.admin_add_scrub_term(&a, &h, "x"), "scrub add");
    sites.check(g.admin_remove_scrub_term(&a, &h), "scrub remove");
    sites.check(g.admin_set_ssh_key(&a, &h, "bob", true), "ssh key label");
    sites.check(g.admin_run_create(&a, &h), "run create");
    sites.check(g.admin_run_end(&a, &h, None), "run end");
    sites.check(g.anchor_done(&a, &h), "anchor done");
    sites.check(
        g.claim(&alice(), "r", &ClaimSpec::new("p", 60, "why").anchored(&h)),
        "claim anchor",
    );
    sites.check(g.admin_set_cap(&a, &h), "unknown label (cap)");
    sites.check(g.check_sealable(&h), "unknown label (seal)");
    let spec = CredentialSpec {
        member: &h,
        clearance: "internal",
        ttl: 60,
        max_life: None,
        parent: None,
    };
    sites.check(g.admin_issue_credential(&a, &spec), "worker name");
    sites.finish();
}

#[test]
fn a_plain_bad_name_keeps_its_text() {
    let g = open();
    let a = admin();
    let t = |r: Result<()>| r.unwrap_err().to_string();
    assert_eq!(
        t(g.admin_remove_member(&a, "r", "bad name")),
        "bad member name: bad name"
    );
    assert_eq!(
        t(g.admin_add_scrub_term(&a, "bad name", "x")),
        "bad rule name: bad name"
    );
    assert_eq!(
        t(g.invite(&alice(), "bob", "bad room", "r")),
        "bad room name: bad room"
    );
    assert_eq!(
        t(g.admin_run_create(&a, "bad id")),
        "a run id is 1 to 32 characters of letters, digits, dash and underscore: \"bad id\""
    );
    assert_eq!(
        t(g.admin_set_ssh_key(&a, "bad label", "bob", true)),
        "bad ssh key label: \"bad label\"; use 1 to 40 letters, digits, '.', '_' or '-'"
    );
}

#[test]
fn a_label_chain_error_is_bounded_and_ascii() {
    let h = hostile_text();
    let e = crate::domain::LabelChain::new(&["a", h.as_str()]).unwrap_err();
    let mut sites = Sites::default();
    sites.check(Err::<(), _>(Error::Invalid(e)), "label chain");
    sites.finish();
    assert_eq!(
        crate::domain::LabelChain::new(&["a", "b c"]).unwrap_err(),
        "bad label name: b c"
    );
}

// ---- the guard: no new raw echo of a caller name in an error (#1768) ----

/// The names a caller types. An error that formats one of these must pass it through `shown`, unless the site is in
/// `ALLOWED` with a reason.
const NAMEISH: [&str; 14] = [
    "name", "member", "room", "via", "to", "from", "label", "key", "id", "anchor", "event",
    "method", "owner", "path",
];

/// (file suffix, name, why a raw echo there is safe). Each is a value that has already passed `valid_name`,
/// `valid_label`, `check_room`, `check_path` or `known_label` (a closed, short alphabet) before the error is built, or
/// a number.
#[rustfmt::skip]
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "gateway/rooms.rs",
        "member",
        "the invite member passed valid_name above",
    ),
    (
        "gateway/rooms.rs",
        "via",
        "the via room passed valid_name above",
    ),
    ("gateway/crossing.rs", "to", "check_room(to) ran first"),
    (
        "gateway/crossing.rs",
        "to_label",
        "a stored label of a known room",
    ),
    (
        "gateway/crossing.rs",
        "label",
        "a stored label of a known room",
    ),
    ("gateway/crossing.rs", "from", "check_room(from) ran first"),
    ("gateway/tripwire.rs", "room", "check_room(room) ran first"),
    (
        "gateway/runs.rs",
        "id",
        "valid_run_id ran first (open and not-open errors)",
    ),
    (
        "gateway/ssh_key.rs",
        "label",
        "valid_label ran first (the mapped and unmapped errors)",
    ),
    ("gateway/ssh_key.rs", "member", "valid_name ran first"),
    (
        "gateway/ssh_key.rs",
        "other",
        "a member name read back from the policy table, stored only after valid_name",
    ),
    (
        "gateway/policy.rs",
        "label",
        "known_label ran first (the sealable and cap errors)",
    ),
    (
        "gateway/scrub.rs",
        "name",
        "valid_name ran first (the no-such-rule error)",
    ),
    (
        "gateway/admin.rs",
        "member",
        "valid_name ran first (the not-removed error)",
    ),
    (
        "gateway/admin.rs",
        "room",
        "check_room ran first (the not-removed error)",
    ),
    (
        "gateway/workers.rs",
        "member",
        "valid_name ran first (the no-live-credential error)",
    ),
    ("board/receipt.rs", "room", "admit ran check_room first"),
    ("gateway/dm.rs", "room", "check_room ran first"),
    ("gateway/policy.rs", "room", "check_room ran first (the clearance and label refusals)"),
    ("gateway/policy.rs", "room_label", "a label read from the store"),
    ("gateway/reviewer.rs", "id", "a numeric queue id"),
    ("gateway/room_require.rs", "room", "check_room ran first"),
    ("gateway/rooms.rs", "room", "valid_name ran first (the invite errors)"),
    ("gateway/rooms.rs", "label", "a label read from the store"),
    ("gateway/runs.rs", "room", "a room read from the run's own rooms"),
    ("gateway/scrub.rs", "room", "check_room ran first (the refusal text)"),
    ("gateway/scrub.rs", "label", "a label read from the store"),
    ("domain/error.rs", "owner", "a member read from a stored claim, valid_name when it was made"),
    ("domain/error.rs", "path", "a claim path that passed check_path when it was made"),
];

/// The identifiers a format string reads: `{ident}` and `{ident:?}` inline, and bare identifiers among the arguments.
fn format_reads(stmt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = stmt;
    while let Some(i) = rest.find('{') {
        rest = &rest[i + 1..];
        if rest.starts_with('{') {
            rest = &rest[1..];
            continue;
        }
        let end = rest.find(['}', ':']).unwrap_or(0);
        let id = &rest[..end];
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push(id.to_string());
        }
    }
    // a bare identifier argument: `, name)` or `, name,`
    for part in stmt.split(", ").skip(1) {
        let id: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let after = &part[id.len()..];
        if !id.is_empty()
            && (after.starts_with(')') || after.starts_with(',') || after.starts_with('\n'))
        {
            out.push(id);
        }
    }
    out
}

/// The offending (file, name) pairs found in `src`, an error statement being everything from an error constructor to the
/// next semicolon.
fn raw_echoes(file: &str, src: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let src = src.split("#[cfg(test)]").next().unwrap_or(src);
    for ctor in ["Error::Invalid(", "Error::NotFound(", "reason: format!("] {
        for (i, _) in src.match_indices(ctor) {
            let stmt = src[i..].split(';').next().unwrap_or("");
            // what is wrapped in `shown(..)` is safe: drop those calls before reading the identifiers
            let mut s = stmt.to_string();
            while let Some(a) = s.find("shown(") {
                let close = s[a..].find(')').map_or(s.len(), |c| a + c + 1);
                s.replace_range(a..close, "");
            }
            for id in format_reads(&s) {
                let base = id.as_str();
                let nameish = NAMEISH.contains(&base) || base.ends_with("_label");
                let allowed = ALLOWED
                    .iter()
                    .any(|(f, n, _)| file.ends_with(f) && *n == base);
                if nameish && !allowed {
                    found.push((file.to_string(), id));
                }
            }
        }
    }
    found
}

fn source_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            source_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && !p.file_stem().unwrap().to_string_lossy().ends_with("_tests")
        {
            out.push(p);
        }
    }
}

#[test]
fn no_error_in_the_gateway_formats_a_raw_caller_name() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for d in ["gateway", "session", "keys", "domain"] {
        source_files(&root.join(d), &mut files);
    }
    let mut found = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        found.extend(raw_echoes(&f.to_string_lossy(), &src));
    }
    assert!(
        found.is_empty(),
        "a caller name is formatted raw in an error; wrap it in `shown(..)` or, if it has already passed a validator, \
         add it to ALLOWED with the reason: {found:?}"
    );
}

#[test]
fn the_guard_sees_a_raw_echo_and_lets_a_shown_one_and_an_allowed_one_pass() {
    let raw = "fn f() { return Err(Error::Invalid(format!(\"bad thing: {member}\"))); }";
    assert_eq!(raw_echoes("gateway/x.rs", raw).len(), 1);
    let raw_arg = "fn f() { return Err(Error::Invalid(format!(\"bad thing: {}\", name))); }";
    assert_eq!(raw_echoes("gateway/x.rs", raw_arg).len(), 1);
    let debug = "Err(Error::Invalid(format!(\"bad: {label:?}\")))";
    assert_eq!(raw_echoes("gateway/x.rs", debug).len(), 1);
    let ok = "Err(Error::Invalid(format!(\"bad thing: {}\", shown(member))));";
    assert!(raw_echoes("gateway/x.rs", ok).is_empty());
    let allowed = "Err(Error::NotFound(format!(\"room {room} has no canary\")));";
    assert!(raw_echoes("gateway/tripwire.rs", allowed).is_empty());
    assert_eq!(raw_echoes("gateway/other.rs", allowed).len(), 1);
    let number = "Err(Error::Invalid(format!(\"no message {seq}\")));";
    assert!(raw_echoes("gateway/x.rs", number).is_empty());
    // test code after the cfg(test) marker is not read
    let test_only = "#[cfg(test)] Err(Error::Invalid(format!(\"{member}\")));";
    assert!(raw_echoes("gateway/x.rs", test_only).is_empty());
}
