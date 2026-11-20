//! Features that need a credential the bench issued in its own temporary store, or a harness the gateway verified
//! (#1049, follows #1045). The credential is issued by `World::issue_worker` through the launcher command, inside the
//! world's temporary home and key directory; nothing of the live keychain or `~/.klatch` is touched.
use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_worker_whose_credential_ended_cannot_touch_its_own_record",
            origin: "#835",
            layer: "functional",
            run: own_record_after_end,
        },
        Scenario {
            name: "the_release_audit_names_who_released_a_crossing",
            origin: "#836, #679",
            layer: "functional",
            run: release_names_releaser,
        },
        Scenario {
            name: "worker_list_shows_the_cap_and_the_last_renewal_and_renewal_never_passes_the_cap",
            origin: "#948, #1007",
            layer: "functional",
            run: worker_list,
        },
        Scenario {
            name: "a_verified_member_admitted_in_flag_mode_is_marked_advisory_on_who_read_history_and_stream",
            origin: "#878, #990, #1032",
            layer: "functional",
            run: advisory_mark,
        },
    ]
}

/// The own-record commands a worker may use while its credential is live, each refused once it ended.
const OWN_RECORD: &[&[&str]] = &[&["crossings"], &["refusals"], &["scope", "repo"]];

fn own_record_after_end(w: &World) -> Res {
    w.set_clock(T0);
    let revoked = w.issue_worker("w-rev", "internal", (600, None))?;
    let idle = w.issue_worker("w-idle", "internal", (5, None))?;
    for bundle in [&revoked, &idle] {
        w.as_worker(bundle, &["join", "r"]);
        for args in OWN_RECORD {
            let r = w.as_worker(bundle, args);
            check(
                r.ok,
                format!("{args:?} refused a live credential: {}", r.stderr),
            )?;
        }
    }
    check(
        w.run("launcher", &["worker", "revoke", "w-rev"]).ok,
        "revoke failed",
    )?;
    w.advance(30);
    for (bundle, word) in [(&revoked, "revoked"), (&idle, "expired")] {
        for args in OWN_RECORD {
            let r = w.as_worker(bundle, args);
            check(
                !r.ok,
                format!("{args:?} was served after the credential ended ({word})"),
            )?;
            has(
                &r.stderr,
                word,
                "the refusal does not say how the credential ended",
            )?;
        }
    }
    // the door is not the only check: a process that proved its credential at the start and stays up (the wire) is
    // refused on its own record once the credential ends under it (#835)
    own_record_on_a_live_wire(w)?;
    // a typed name is not a credential: the same commands as the same member name are never refused for it
    check(
        w.run("w-rev", &["crossings"]).ok,
        "a typed name was refused by the own-record check",
    )?;
    Ok(vec![])
}

/// A worker's `serve --stdio` keeps running while the launcher revokes its credential: every own-record method it
/// calls afterwards is refused with the `credential` rule, though the same call before the revoke was served.
fn own_record_on_a_live_wire(w: &World) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    let bundle = w.issue_worker("w-wire", "internal", (600, None))?;
    let mut child = w
        .spawn_worker_piped(&bundle, &["serve", "--stdio"])
        .map_err(|e| format!("start serve: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut out = BufReader::new(child.stdout.take().ok_or("no stdout")?);
    // the server greets with who it took the caller to be before it reads a request
    let mut hello = String::new();
    out.read_line(&mut hello).map_err(|e| e.to_string())?;
    has(&hello, "\"verified\"", "the wire did not verify the worker")?;
    let mut call = |id: &str, method: &str, params: serde_json::Value| -> Result<String, String> {
        let req = serde_json::json!({"v": 1, "type": "request", "id": id, "method": method, "params": params});
        writeln!(stdin, "{req}").map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        out.read_line(&mut line).map_err(|e| e.to_string())?;
        Ok(line)
    };
    let live = call("1", "my_crossings", serde_json::json!({}));
    let revoked = w.run("launcher", &["worker", "revoke", "w-wire"]).ok;
    let after = [
        call("2", "my_crossings", serde_json::json!({})),
        call("3", "scope", serde_json::json!({"scope": "repo"})),
    ];
    let _ = child.kill();
    let _ = child.wait();
    has(
        &live?,
        "\"ok\":true",
        "a live credential was refused on the wire",
    )?;
    check(revoked, "revoke failed")?;
    for line in after {
        let line = line?;
        lacks(
            &line,
            "\"ok\":true",
            "an ended credential was served on its own record",
        )?;
        has(
            &line,
            "the credential was revoked",
            "the refusal does not say the credential ended",
        )?;
    }
    Ok(())
}

fn release_names_releaser(w: &World) -> Res {
    w.set_clock(T0);
    check(
        w.admin(&["label", "vault", "private"], "").ok,
        "label failed",
    )?;
    check(
        w.admin(&["clear", "alice", "private"], "").ok,
        "clear failed",
    )?;
    check(
        w.admin(&["reviewer", "rev"], "").ok,
        "granting the reviewer role failed",
    )?;
    check(
        w.admin(&["clear", "rev", "private"], "").ok,
        "clearing the reviewer failed",
    )?;
    w.ok("alice", &["join", "vault"])?;
    w.ok("alice", &["join", "open"])?;
    w.ok("bob", &["join", "open"])?;
    // seq 1 of a private room is its canary (#347)
    w.ok("alice", &["post", "vault", "the plan is blue"])?;
    w.ok("alice", &["post", "vault", "deploy to quokka.internal"])?;
    w.ok("alice", &["cross", "vault", "2", "--to", "open"])?;
    w.ok("alice", &["cross", "vault", "3", "--to", "open"])?;
    // the reviewer lets the first through; the audit names the reviewer
    has(
        &w.ok("rev", &["review", "verdict", "1", "allow"])?,
        "verdict recorded",
        "the reviewer's verdict",
    )?;
    // a private term added after the request: the floors are checked again at release
    check(
        w.admin(&["scrub-add", "hosts", "quokka.internal"], "").ok,
        "scrub-add failed",
    )?;
    // the launcher's judge denies the second (a private term)
    let pass = w.run("launcher", &["judge", "run", "--once"]);
    check(pass.ok, format!("judge run failed: {}", pass.stderr))?;
    let audit = w.admin(&["audit"], "").stdout;
    check(
        audit.lines().any(|l| {
            l.contains("crossing") && l.contains("vault#2") && l.contains("applied by rev")
        }),
        format!("the reviewer is not named as the releaser:\n{audit}"),
    )?;
    // the judge's own deny is audited as the judge's and releases nothing; naming the judge as a releaser needs an
    // allow, which only the opt-in hosted judge (a `claude` binary of the operator's) gives, so the bench leaves it
    check(
        audit
            .lines()
            .any(|l| l.contains("judge") && l.contains("vault#3") && l.contains("deny")),
        format!("the judge's deny is not in the audit:\n{audit}"),
    )?;
    lacks(
        &audit,
        "vault#3\treleased",
        "a denied crossing was released",
    )?;
    lacks(&audit, "quokka", "the audit holds the text")?;
    // the released text arrived, marked as a crossing from the source (#679); the denied one did not
    let open = w.ok("bob", &["read", "open"])?;
    has(
        &open,
        "the plan is blue",
        "the released text did not arrive",
    )?;
    has(&open, "vault#2", "the handoff does not name the source")?;
    lacks(&open, "quokka", "a denied crossing arrived")?;
    let mine = w.ok("alice", &["crossings"])?;
    has(&mine, "released", "the released state")?;
    has(&mine, "denied", "the denied state")?;
    Ok(vec![])
}

fn worker_list(w: &World) -> Res {
    w.set_clock(T0);
    let bundle = w.issue_worker("w-cap", "internal", (600, Some(900)))?;
    let line = |w: &World| -> Result<String, String> {
        let l = w.ok("launcher", &["worker", "list"])?;
        l.lines()
            .find(|l| l.starts_with("w-cap\t"))
            .map(String::from)
            .ok_or_else(|| format!("w-cap is not listed:\n{l}"))
    };
    let first = line(w)?;
    has(&first, "cap in 900s", "the cap at issue")?;
    has(&first, "expires in 600s", "the idle timeout at issue")?;
    has(&first, "last renewed never", "no renewal yet")?;
    w.advance(100);
    check(
        w.as_worker(&bundle, &["post", "r", "one"]).ok,
        "the worker could not post",
    )?;
    let used = line(w)?;
    has(&used, "last renewed 0s ago", "the renewal on use")?;
    has(&used, "expires in 600s", "the timeout restarts on use")?;
    has(&used, "cap in 800s", "the cap counts down")?;
    w.advance(50);
    let idle = line(w)?;
    has(&idle, "last renewed 50s ago", "the time since the renewal")?;
    // late in life: renewal moves the expiry only up to the cap
    w.advance(400);
    check(
        w.as_worker(&bundle, &["post", "r", "two"]).ok,
        "the worker could not post near the cap",
    )?;
    let late = line(w)?;
    has(&late, "cap in 350s", "the cap")?;
    has(
        &late,
        "expires in 350s",
        "renewal passed the cap (or did not renew)",
    )?;
    // after the cap the credential is over, whatever its timeout
    w.advance(400);
    has(
        &w.as_worker(&bundle, &["post", "r", "three"]).stderr,
        "expired",
        "a worker past its cap still posted",
    )?;
    Ok(vec![])
}

fn advisory_mark(w: &World) -> Res {
    w.set_clock(T0);
    // a member who joined before the room asked for anything is never flagged
    w.ok("early", &["join", "flagged"])?;
    w.ok(
        "early",
        &["post", "flagged", "hello before the requirement"],
    )?;
    // a room that asks for a capability Claude Code only has at a degraded level, in flag mode: admitted, but flagged
    check(
        w.admin(&["room-require", "flagged", "flag:permission-mode"], "")
            .ok,
        "room-require failed",
    )?;
    check(
        w.admin(
            &["room-require", "native", "flag:pre-action-enforcement"],
            "",
        )
        .ok,
        "room-require failed",
    )?;
    let h = FakeHarness::new(w)?;
    let s = h.start("adv-sid-0001", "adv")?;
    let (ok, out) = s.klatch(&["join", "flagged"])?;
    check(ok, format!("the verified session was not admitted: {out}"))?;
    let (ok, out) = s.klatch(&["post", "flagged", "hello from a flagged harness"])?;
    check(ok, format!("the flagged session could not post: {out}"))?;
    let who = w.ok("obs", &["who", "flagged", "--all"])?;
    let line_of = |name: &str| who.lines().find(|l| l.starts_with(name)).unwrap_or("");
    has(
        line_of("adv"),
        "advisory",
        &format!("who does not mark the flagged member in:\n{who}"),
    )?;
    lacks(
        line_of("early"),
        "advisory",
        "who marks a member admitted before the requirement",
    )?;
    let read = w.ok("obs", &["read", "flagged", "--peek"])?;
    marked(&read, "read")?;
    let history = w.ok("obs", &["history", "flagged"])?;
    marked(&history, "history")?;
    // the live stream carries it as a field of the message frame (#1032)
    let stream = w.ok("obs", &["stream", "flagged", "--from-seq", "0"])?;
    let frame = |needle: &str| stream.lines().find(|l| l.contains(needle)).unwrap_or("");
    has(
        frame("flagged harness"),
        "\"advisory\":true",
        &format!("the stream frame does not carry the mark in:\n{stream}"),
    )?;
    lacks(
        frame("before the requirement"),
        "advisory",
        "the stream marks a clean sender",
    )?;
    // the harness the gateway verified has the capability a second room asks for, so it is admitted clean there; a
    // caller that names no harness stays at the generic floor and is flagged for the same requirement
    let (ok, out) = s.klatch(&["join", "native"])?;
    check(ok, format!("the verified session was not admitted: {out}"))?;
    w.ok("typed", &["join", "native"])?;
    let who = w.ok("obs", &["who", "native", "--all"])?;
    let line_of = |name: &str| who.lines().find(|l| l.starts_with(name)).unwrap_or("");
    lacks(
        line_of("adv"),
        "advisory",
        "a verified harness with the capability was flagged",
    )?;
    has(
        line_of("typed"),
        "advisory",
        &format!("a caller that names no harness was not flagged in:\n{who}"),
    )?;
    Ok(vec![])
}

/// The flagged sender's line carries the mark and an unflagged member's line does not.
fn marked(text: &str, what: &str) -> Result<(), String> {
    let adv = text
        .lines()
        .find(|l| l.contains("flagged harness"))
        .ok_or_else(|| format!("{what}: the flagged message is missing:\n{text}"))?;
    has(
        adv,
        "(advisory)",
        &format!("{what} does not mark the sender"),
    )?;
    let plain = text
        .lines()
        .find(|l| l.contains("before the requirement"))
        .ok_or_else(|| format!("{what}: the early member's message is missing:\n{text}"))?;
    lacks(plain, "advisory", &format!("{what} marks a clean sender"))
}
