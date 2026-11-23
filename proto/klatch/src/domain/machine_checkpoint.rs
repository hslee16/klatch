//! The machine checkpoint (#1408): a compact record of where a member's session stands, written by the Stop and
//! SessionEnd hooks (opt in) beside the durable task, so a successor or the same member after a resume loses nothing
//! it could not have said itself. It is data about work, never the work: tool classes, repo-relative paths of files
//! edited or written, claim paths, ask numbers and the last status word. Never a command line, file content, tool
//! output or message text. One per member and room, replaced and never appended, and at most `MACHINE_MAX_BYTES`.

/// The most bytes one machine checkpoint holds. 600 bytes is about 150 tokens: room for eight or so file paths and the
/// rest of the line, small enough that a task line or a briefing that shows it costs little context, and large enough
/// that a successor can tell what was touched without reading the log.
pub const MACHINE_MAX_BYTES: usize = 600;
/// The distinct files a trail keeps (the latest touched last) and the tool classes it keeps (the latest last).
pub const TRAIL_FILES_MAX: usize = 24;
pub const TRAIL_TOOLS_MAX: usize = 8;
/// The longest path a trail keeps; a longer one is not recorded (it is not a path a person can use from a line).
pub const TRAIL_PATH_MAX_BYTES: usize = 120;
/// The mark that tells a reader a checkpoint came from a hook and not from the member.
pub const MACHINE_MARK: &str = "machine checkpoint";

/// What a tool call was, from a closed vocabulary. Never the tool's own name (an MCP tool's name is the user's text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    Edit,
    Write,
    Run,
    Read,
    Other,
}

impl ToolClass {
    pub fn word(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Write => "write",
            Self::Run => "run",
            Self::Read => "read",
            Self::Other => "other",
        }
    }

    /// The class of a tool by its name; a name outside the table is `Other`.
    pub fn of_tool(name: &str) -> Self {
        match name {
            "Edit" | "MultiEdit" | "NotebookEdit" | "apply_patch" => Self::Edit,
            "Write" => Self::Write,
            "Bash" | "shell" | "exec_command" => Self::Run,
            "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" => Self::Read,
            _ => Self::Other,
        }
    }

    /// Only an edit or a write names a file worth recording.
    pub fn keeps_paths(self) -> bool {
        matches!(self, Self::Edit | Self::Write)
    }
}

/// `path` as it is written under `root` (both absolute, already cleaned), or none when it lies outside `root`.
/// A path with a control character, or longer than `TRAIL_PATH_MAX_BYTES`, is not kept.
pub fn repo_relative(path: &str, root: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    if rest.is_empty() || rest.len() > TRAIL_PATH_MAX_BYTES || rest.chars().any(char::is_control) {
        return None;
    }
    Some(rest.to_string())
}

/// What a member has done since its trail was last cleared: the distinct files it edited or wrote and the classes of
/// its latest tool calls. Held between hooks so the Stop hook can write them down.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trail {
    pub files: Vec<String>,
    pub tools: Vec<String>,
}

impl Trail {
    /// Add one tool call. A file touched again moves to the end; the oldest fall off at the caps.
    pub fn push(&mut self, class: ToolClass, paths: &[String]) {
        self.tools.push(class.word().to_string());
        if self.tools.len() > TRAIL_TOOLS_MAX {
            self.tools.remove(0);
        }
        if !class.keeps_paths() {
            return;
        }
        for p in paths {
            self.files.retain(|f| f != p);
            self.files.push(p.clone());
        }
        let extra = self.files.len().saturating_sub(TRAIL_FILES_MAX);
        self.files.drain(..extra);
    }
}

/// How a session ended, a word from a closed list (the SessionEnd `reason`); anything else is `other`.
pub fn end_word(reason: &str) -> &'static str {
    match reason {
        "clear" => "clear",
        "resume" => "resume",
        "logout" => "logout",
        "prompt_input_exit" => "exit",
        _ => "other",
    }
}

/// What a checkpoint says, before it is written as a line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineState {
    pub files: Vec<String>,
    pub tools: Vec<String>,
    /// Claim paths the member holds in the room.
    pub claims: Vec<String>,
    /// Asks the member raised that are open, and of those the ones waiting on the member's own answer.
    pub asks_open: Vec<i64>,
    pub asks_waiting: Vec<i64>,
    /// The first word of the member's last status.
    pub status: Option<String>,
    /// `None` at a Stop; the end word at a SessionEnd.
    pub ended: Option<&'static str>,
}

/// The first word of a status text, kept to plain characters and 24 of them.
pub fn status_word(status: &str) -> Option<String> {
    let w: String = status
        .split_whitespace()
        .next()?
        .trim_end_matches(':')
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    (!w.is_empty()).then_some(w)
}

fn clip(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn join_numbers(prefix: &str, ns: &[i64]) -> String {
    let list: Vec<String> = ns.iter().map(|n| format!("#{n}")).collect();
    format!("{prefix} {}", list.join(" "))
}

/// The one-line body of a checkpoint, at most `MACHINE_MAX_BYTES`. Parts go in order of what a successor needs most:
/// the end word, claims, asks, status, files, then the tool classes. Files are the part that shrinks (the latest
/// first, the rest counted as `+N`); a line still too long is cut at a character boundary.
pub fn machine_body(s: &MachineState) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match s.ended {
        Some(w) => format!("session ended ({w})"),
        None => "turn ended".to_string(),
    });
    if !s.claims.is_empty() {
        parts.push(format!("claims {}", s.claims.join(" ")));
    }
    if !s.asks_open.is_empty() {
        parts.push(join_numbers("asks open", &s.asks_open));
    }
    if !s.asks_waiting.is_empty() {
        parts.push(join_numbers("waiting on me", &s.asks_waiting));
    }
    if let Some(w) = &s.status {
        parts.push(format!("status {w}"));
    }
    let tools = (!s.tools.is_empty()).then(|| format!("tools {}", s.tools.join(" ")));
    let line = |parts: &[String], files: &str, tools: &Option<String>| {
        let mut all = parts.to_vec();
        if !files.is_empty() {
            all.push(files.to_string());
        }
        all.extend(tools.clone());
        all.join("; ")
    };
    // newest file first; drop from the oldest until the line fits
    let newest: Vec<&String> = s.files.iter().rev().collect();
    for keep in (0..=newest.len()).rev() {
        let files = if newest.is_empty() {
            String::new()
        } else {
            let shown: Vec<&str> = newest[..keep].iter().map(|f| f.as_str()).collect();
            let more = newest.len() - keep;
            match (keep, more) {
                (0, m) => format!("files +{m}"),
                (_, 0) => format!("files {}", shown.join(" ")),
                (_, m) => format!("files {} +{m}", shown.join(" ")),
            }
        };
        let text = line(&parts, &files, &tools);
        if text.len() <= MACHINE_MAX_BYTES {
            return text;
        }
    }
    // even with no file listed it is too long: drop the tools, then cut
    let text = line(&parts, "", &None);
    clip(&text, MACHINE_MAX_BYTES).to_string()
}

/// A machine checkpoint as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineCheckpoint {
    pub room: String,
    pub member: String,
    pub body: String,
    pub ts: i64,
    /// Written by a SessionEnd (the last one the session will write).
    pub last: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_fall_into_a_closed_vocabulary_and_only_edits_and_writes_keep_paths() {
        assert_eq!(ToolClass::of_tool("Edit"), ToolClass::Edit);
        assert_eq!(ToolClass::of_tool("MultiEdit"), ToolClass::Edit);
        assert_eq!(ToolClass::of_tool("Write"), ToolClass::Write);
        assert_eq!(ToolClass::of_tool("Bash"), ToolClass::Run);
        assert_eq!(ToolClass::of_tool("Grep"), ToolClass::Read);
        assert_eq!(ToolClass::of_tool("mcp__secret__tool"), ToolClass::Other);
        assert!(ToolClass::Edit.keeps_paths() && ToolClass::Write.keeps_paths());
        assert!(!ToolClass::Run.keeps_paths() && !ToolClass::Read.keeps_paths());
    }

    #[test]
    fn a_path_is_kept_only_under_the_root_and_only_when_plain() {
        assert_eq!(
            repo_relative("/w/p/src/a.rs", "/w/p").as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(repo_relative("/w/other/a.rs", "/w/p"), None);
        assert_eq!(repo_relative("/w/p", "/w/p"), None);
        assert_eq!(repo_relative("/w/pp/a.rs", "/w/p"), None);
        assert_eq!(repo_relative("/w/p/a\nb", "/w/p"), None);
        assert_eq!(
            repo_relative(&format!("/w/p/{}", "x".repeat(200)), "/w/p"),
            None
        );
    }

    #[test]
    fn a_trail_keeps_distinct_files_latest_last_and_the_latest_tool_classes() {
        let mut t = Trail::default();
        t.push(ToolClass::Edit, &["a.rs".into()]);
        t.push(ToolClass::Write, &["b.rs".into()]);
        t.push(ToolClass::Edit, &["a.rs".into()]);
        t.push(ToolClass::Run, &["ignored.rs".into()]);
        assert_eq!(t.files, vec!["b.rs", "a.rs"]);
        assert_eq!(t.tools, vec!["edit", "write", "edit", "run"]);
        for i in 0..40 {
            t.push(ToolClass::Edit, &[format!("f{i}.rs")]);
        }
        assert_eq!(t.files.len(), TRAIL_FILES_MAX);
        assert_eq!(t.files.last().unwrap(), "f39.rs");
        assert_eq!(t.tools.len(), TRAIL_TOOLS_MAX);
    }

    #[test]
    fn a_session_end_reason_becomes_one_of_a_few_words() {
        assert_eq!(end_word("clear"), "clear");
        assert_eq!(end_word("prompt_input_exit"), "exit");
        assert_eq!(end_word("sudo rm -rf /"), "other");
        assert_eq!(
            status_word("blocked: waiting for review").as_deref(),
            Some("blocked")
        );
        assert_eq!(status_word("   "), None);
    }

    #[test]
    fn the_body_names_claims_asks_status_files_and_tools_within_the_cap() {
        let s = MachineState {
            files: vec!["src/a.rs".into(), "src/b.rs".into()],
            tools: vec!["read".into(), "edit".into()],
            claims: vec!["src/a.rs".into()],
            asks_open: vec![12],
            asks_waiting: vec![14],
            status: Some("working".into()),
            ended: None,
        };
        let b = machine_body(&s);
        assert_eq!(
            b,
            "turn ended; claims src/a.rs; asks open #12; waiting on me #14; status working; files src/b.rs src/a.rs; tools read edit"
        );
        let end = machine_body(&MachineState {
            ended: Some("clear"),
            ..s
        });
        assert!(end.starts_with("session ended (clear);"));
    }

    #[test]
    fn a_body_over_the_cap_drops_the_oldest_files_and_counts_them() {
        let s = MachineState {
            files: (0..24)
                .map(|i| format!("crates/deep/path/to/module_{i:02}.rs"))
                .collect(),
            tools: vec!["edit".into(); 8],
            claims: vec!["crates/deep/path/to/module_23.rs".into()],
            ..MachineState::default()
        };
        let b = machine_body(&s);
        assert!(b.len() <= MACHINE_MAX_BYTES, "{} bytes", b.len());
        assert!(b.contains("module_23.rs"), "{b}");
        assert!(!b.contains("module_00.rs"), "{b}");
        assert!(b.contains(" +"), "the dropped files are counted: {b}");
    }

    #[test]
    fn a_body_stays_in_the_cap_whatever_the_parts_are() {
        let s = MachineState {
            files: vec!["é".repeat(100)],
            claims: (0..40)
                .map(|i| format!("claim/{}/{i}", "é".repeat(30)))
                .collect(),
            asks_open: (0..200).collect(),
            status: Some("x".repeat(24)),
            ..MachineState::default()
        };
        let b = machine_body(&s);
        assert!(b.len() <= MACHINE_MAX_BYTES);
        assert!(!b.is_empty());
    }
}
