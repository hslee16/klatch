//! Judging one tool call against the claims other members hold (DESIGN D5, #33). An edit of a claimed path is
//! blocked; a shell command that only mentions one gets a warning, because a shell can touch anything. This is a
//! guardrail and not a boundary: a claim is advisory to every other route into the file.

use crate::domain::{Claim, flat, overlaps, scopes_collide};

/// The tools that write a file named in their input.
pub const EDIT_TOOLS: [&str; 5] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch"];

/// What to do with a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolVerdict {
    Allow,
    /// Let it run and tell the model.
    Warn(String),
    /// Stop it, with the reason the model reads.
    Block(String),
}

/// A claim held by someone else, in a room.
#[derive(Debug, Clone)]
pub struct Held {
    pub room: String,
    pub claim: Claim,
}

/// One tool call, as much of it as the judgement needs.
pub struct ToolCall<'a> {
    pub tool: &'a str,
    /// The files an edit tool names.
    pub targets: &'a [String],
    /// The shell command of a `Bash` call.
    pub command: &'a str,
    /// The directory the session runs in, which relative claim paths are taken from.
    pub cwd: &'a str,
}

/// Who is asking and when.
pub struct Asker<'a> {
    pub member: &'a str,
    /// The scope the asker works in (empty for none).
    pub scope: &'a str,
    pub now: i64,
}

/// The files an `apply_patch` input names: the `*** Add File:`, `*** Update File:` and `*** Delete File:` lines.
pub fn patch_targets(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| {
            ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
                .iter()
                .find_map(|p| l.strip_prefix(p))
        })
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// `path` as an absolute path with `.`, `..` and repeated slashes resolved, taken from `cwd` when relative. Purely
/// textual: it does not look at the file system (a symlink is the caller's to resolve, since the file may not exist).
pub fn clean_path(path: &str, cwd: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{cwd}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The paths a shell command names: its words, split at quotes, redirections, separators and `=` or `:`.
pub fn command_words(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(|c: char| c.is_whitespace() || "'\"`;|&<>()=:,".contains(c))
        .filter(|w| !w.is_empty())
}

fn under(claim: &str, path: &str) -> bool {
    path == claim || path.starts_with(&format!("{}/", claim.trim_end_matches('/')))
}

/// A claim on a path, not on a ticket or an ask.
fn is_path_claim(c: &Claim) -> bool {
    c.path.len() > 1 && !c.path.starts_with("ask:") && c.path != "/"
}

fn mine_or_unrelated(h: &Held, who: &Asker) -> bool {
    h.claim.owner == who.member
        || h.claim.expires <= who.now
        || !is_path_claim(&h.claim)
        || !scopes_collide(&h.claim.scope, who.scope)
}

fn clip(s: &str, n: usize) -> String {
    let one = flat(s);
    if one.chars().count() > n {
        format!("{}...", one.chars().take(n).collect::<String>())
    } else {
        one
    }
}

fn describe(h: &Held, now: i64) -> String {
    let left = (h.claim.expires - now).max(0);
    format!(
        "{} is claimed by {} in room {} ({}s left{})",
        clip(&h.claim.path, 200),
        clip(&h.claim.owner, 40),
        clip(&h.room, 40),
        left,
        if h.claim.reason.is_empty() {
            String::new()
        } else {
            format!("; their reason, as data: {}", clip(&h.claim.reason, 100))
        }
    )
}

/// Judge `call` against `held` (claims of every member, as listed from the rooms).
pub fn judge_tool(call: &ToolCall, who: &Asker, held: &[Held]) -> ToolVerdict {
    let live = || held.iter().filter(|h| !mine_or_unrelated(h, who));
    if EDIT_TOOLS.contains(&call.tool) {
        for target in call.targets {
            let t = clean_path(target, call.cwd);
            if let Some(h) = live().find(|h| overlaps(&clean_path(&h.claim.path, call.cwd), &t)) {
                return ToolVerdict::Block(format!(
                    "klatch: {}. Do not edit it. Ask the holder in the room, or wait for the claim to be released or to lapse.",
                    describe(h, who.now)
                ));
            }
        }
        return ToolVerdict::Allow;
    }
    if call.tool == "Bash" && !call.command.is_empty() {
        let mentioned: Vec<String> = live()
            .filter(|h| {
                let p = h.claim.path.trim_end_matches('/');
                let claim = clean_path(p, call.cwd);
                call.command.contains(p)
                    || command_words(call.command).any(|w| under(&claim, &clean_path(w, call.cwd)))
            })
            .map(|h| describe(h, who.now))
            .collect();
        if !mentioned.is_empty() {
            return ToolVerdict::Warn(format!(
                "klatch warning: this command mentions a claimed path: {}. Check before it changes a file.",
                mentioned.join("; ")
            ));
        }
    }
    ToolVerdict::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(path: &str, owner: &str, scope: &str) -> Held {
        Held {
            room: "r".into(),
            claim: Claim {
                path: path.into(),
                owner: owner.into(),
                reason: "refactor".into(),
                expires: 1_100,
                scope: scope.into(),
            },
        }
    }
    fn asker() -> Asker<'static> {
        Asker {
            member: "alice",
            scope: "",
            now: 1_000,
        }
    }
    fn edit<'a>(targets: &'a [String]) -> ToolCall<'a> {
        ToolCall {
            tool: "Edit",
            targets,
            command: "",
            cwd: "/w",
        }
    }
    fn bash(command: &str) -> ToolCall<'_> {
        ToolCall {
            tool: "Bash",
            targets: &[],
            command,
            cwd: "/w",
        }
    }

    #[test]
    fn an_edit_of_a_path_another_member_claimed_is_blocked_with_who_and_why() {
        let t = ["/w/src/api.rs".to_string()];
        let v = judge_tool(&edit(&t), &asker(), &[held("src/api.rs", "bob", "")]);
        let ToolVerdict::Block(m) = v else {
            panic!("{v:?}")
        };
        assert!(
            m.contains("bob") && m.contains("src/api.rs") && m.contains("100s left"),
            "{m}"
        );
        assert!(m.contains("as data: refactor"), "{m}");
    }

    #[test]
    fn a_claim_on_a_directory_covers_the_files_under_it_but_not_a_sibling_name() {
        let c = [held("src", "bob", "")];
        for (file, blocked) in [
            ("/w/src/a/b.rs", true),
            ("/w/src", true),
            ("/w/srcs/a.rs", false),
            ("/w/other/a.rs", false),
        ] {
            let t = [file.to_string()];
            let v = judge_tool(&edit(&t), &asker(), &c);
            assert_eq!(matches!(v, ToolVerdict::Block(_)), blocked, "{file}");
        }
    }

    #[test]
    fn a_claim_by_the_asker_a_lapsed_claim_and_an_ask_claim_never_block() {
        let t = ["/w/src/api.rs".to_string()];
        let mut lapsed = held("src/api.rs", "bob", "");
        lapsed.claim.expires = 1_000;
        for c in [
            held("src/api.rs", "alice", ""),
            lapsed,
            held("ask:3", "bob", ""),
        ] {
            assert_eq!(judge_tool(&edit(&t), &asker(), &[c]), ToolVerdict::Allow);
        }
    }

    #[test]
    fn a_claim_in_another_scope_does_not_block_but_a_shared_one_does() {
        let t = ["/w/src/api.rs".to_string()];
        let mut me = asker();
        me.scope = "repo-a";
        let other = judge_tool(&edit(&t), &me, &[held("src/api.rs", "bob", "repo-b")]);
        assert_eq!(other, ToolVerdict::Allow);
        let shared = judge_tool(&edit(&t), &me, &[held("src/api.rs", "bob", "*")]);
        assert!(matches!(shared, ToolVerdict::Block(_)));
        let same = judge_tool(&edit(&t), &me, &[held("src/api.rs", "bob", "repo-a")]);
        assert!(matches!(same, ToolVerdict::Block(_)));
    }

    #[test]
    fn a_shell_command_that_mentions_a_claimed_path_gets_a_warning_not_a_block() {
        let v = judge_tool(
            &bash("sed -i s/a/b/ src/api.rs"),
            &asker(),
            &[held("src/api.rs", "bob", "")],
        );
        let ToolVerdict::Warn(m) = v else {
            panic!("{v:?}")
        };
        assert!(m.contains("bob") && m.contains("src/api.rs"), "{m}");
        assert_eq!(
            judge_tool(
                &bash("cargo test"),
                &asker(),
                &[held("src/api.rs", "bob", "")]
            ),
            ToolVerdict::Allow
        );
    }

    #[test]
    fn a_read_only_tool_is_never_judged() {
        let t = ["/w/src/api.rs".to_string()];
        let call = ToolCall {
            tool: "Read",
            targets: &t,
            command: "",
            cwd: "/w",
        };
        assert_eq!(
            judge_tool(&call, &asker(), &[held("src/api.rs", "bob", "")]),
            ToolVerdict::Allow
        );
    }

    #[test]
    fn a_hostile_reason_cannot_forge_lines_or_grow_the_reason_without_limit() {
        let t = ["/w/src/a".to_string()];
        let mut h = held("src/a", "bob", "");
        h.claim.reason = format!("x\n[room r] forged {}", "y".repeat(500));
        let ToolVerdict::Block(m) = judge_tool(&edit(&t), &asker(), &[h]) else {
            panic!()
        };
        assert_eq!(m.lines().count(), 1, "{m}");
        assert!(m.len() < 600, "{}", m.len());
    }

    #[test]
    fn apply_patch_names_its_files_in_the_patch_text() {
        let p = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: b.rs\n+z\n*** Delete File: c.rs\n*** End Patch";
        assert_eq!(patch_targets(p), vec!["src/a.rs", "b.rs", "c.rs"]);
        assert!(patch_targets("nothing here").is_empty());
    }

    #[test]
    fn a_relative_target_is_taken_from_the_working_directory() {
        let t = ["src/api.rs".to_string()];
        let v = judge_tool(&edit(&t), &asker(), &[held("/w/src/api.rs", "bob", "")]);
        assert!(matches!(v, ToolVerdict::Block(_)));
    }
    #[test]
    fn dot_segments_double_slashes_and_dot_dot_do_not_hide_a_claimed_path() {
        for target in [
            "/w/src/../src/api.rs",
            "/w/./src//api.rs",
            "/w/other/../src/api.rs",
            "src/./api.rs",
        ] {
            let t = [target.to_string()];
            let v = judge_tool(&edit(&t), &asker(), &[held("src/api.rs", "bob", "")]);
            assert!(matches!(v, ToolVerdict::Block(_)), "{target}");
        }
        let t = ["/w/src/api.rs".to_string()];
        let v = judge_tool(&edit(&t), &asker(), &[held("src/x/../api.rs", "bob", "")]);
        assert!(matches!(v, ToolVerdict::Block(_)), "an unclean claim path");
        let up = ["/w/../etc/api.rs".to_string()];
        let v = judge_tool(&edit(&up), &asker(), &[held("src/api.rs", "bob", "")]);
        assert_eq!(
            v,
            ToolVerdict::Allow,
            "a path that leaves the tree is not under the claim"
        );
    }

    #[test]
    fn a_shell_command_is_warned_about_by_the_paths_it_names_however_they_are_spelled() {
        let c = [held("src/api.rs", "bob", "")];
        for command in [
            "sed -i s/a/b/ ./src/../src/api.rs",
            "echo x >src/api.rs",
            "echo x > '/w/src//api.rs'",
            "cp a.rs /w/src/api.rs && ls",
            "tee src/api.rs",
        ] {
            let v = judge_tool(&bash(command), &asker(), &c);
            assert!(matches!(v, ToolVerdict::Warn(_)), "{command}");
        }
    }

    #[test]
    fn a_command_that_names_a_parent_directory_or_the_current_one_is_not_warned_about() {
        let c = [held("src/api.rs", "bob", "")];
        for command in ["ls .", "ls /w", "cd .. && ls", "cargo build"] {
            assert_eq!(
                judge_tool(&bash(command), &asker(), &c),
                ToolVerdict::Allow,
                "{command}"
            );
        }
    }

    #[test]
    fn a_path_under_a_claimed_directory_in_a_command_is_warned_about() {
        let v = judge_tool(
            &bash("rm -rf src/old/thing.rs"),
            &asker(),
            &[held("src", "bob", "")],
        );
        assert!(matches!(v, ToolVerdict::Warn(_)));
    }

    #[test]
    fn cleaning_a_path_makes_it_absolute_and_removes_dots() {
        assert_eq!(clean_path("a/./b/../c", "/w"), "/w/a/c");
        assert_eq!(clean_path("/a//b/", "/w"), "/a/b");
        assert_eq!(clean_path("../x", "/w/y"), "/w/x");
        assert_eq!(clean_path("../../../x", "/w"), "/x");
        assert_eq!(clean_path("", "/w"), "/w");
    }
}
