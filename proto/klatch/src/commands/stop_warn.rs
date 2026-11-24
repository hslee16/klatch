//! The stop warning (#1344): what a Stop hook that carries `--stop-warn` tells the human when a turn ends while
//! background tasks or crons still run. Warn only: nothing is stopped, nothing is written, and no word of a task is
//! ever echoed. Only counts and kinds from a fixed set reach the line.

use serde_json::Value;

/// The kinds a background task is counted under, in the order the line lists them.
const KINDS: [&str; 4] = ["shell", "monitor", "agent", "other"];

/// The fixed kind a task entry counts under. The entry shape is not documented in the repo (ticket 1593), so a string
/// under `type` or else `kind` is matched against fixed words, and anything else (no field, a number, an unknown
/// word) is "other". The word itself is never kept.
fn kind_of(entry: &Value) -> &'static str {
    let word = ["type", "kind"]
        .iter()
        .find_map(|k| entry.get(k).and_then(Value::as_str))
        .unwrap_or("")
        .to_ascii_lowercase();
    match word.as_str() {
        "shell" | "bash" | "local_bash" | "background_shell" => "shell",
        "monitor" => "monitor",
        "agent" | "subagent" | "local_agent" | "background_agent" => "agent",
        _ => "other",
    }
}

/// Counts of the tasks by kind (indexed as `KINDS`) and the number of crons. A list counts its entries, a bare number
/// counts as that many entries of kind "other", and anything else (absent, wrong type) counts as none.
fn tally(input: &Value) -> ([usize; 4], usize) {
    let mut kinds = [0; 4];
    match input.get("background_tasks") {
        Some(Value::Array(list)) => {
            for entry in list {
                let k = kind_of(entry);
                kinds[KINDS.iter().position(|w| *w == k).unwrap_or(3)] += 1;
            }
        }
        Some(Value::Number(n)) => kinds[3] = n.as_u64().map_or(0, |n| n.min(9999) as usize),
        _ => {}
    }
    let crons = match input.get("session_crons") {
        Some(Value::Array(list)) => list.len(),
        Some(Value::Number(n)) => n.as_u64().map_or(0, |n| n.min(9999) as usize),
        _ => 0,
    };
    (kinds, crons)
}

/// The one line for a Stop input, or none when nothing still runs (or the input is not an object).
pub(super) fn warning(input: &Value) -> Option<String> {
    let (kinds, crons) = tally(input);
    let tasks: usize = kinds.iter().sum();
    if tasks + crons == 0 {
        return None;
    }
    let by_kind: Vec<String> = KINDS
        .iter()
        .zip(kinds)
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| format!("{k} {n}"))
        .collect();
    let listed = if by_kind.is_empty() {
        "none".to_string()
    } else {
        by_kind.join(", ")
    };
    Some(format!(
        "klatch: this session stopped with {tasks} background task(s) ({listed}) and {crons} cron(s) still running. Stop them, or start one wait only (klatch wait)."
    ))
}

/// The hook's output: a `systemMessage` JSON object that the human sees, or nothing. The consent is the flag.
pub(super) fn output(input: Option<&Value>, consent: bool) -> Option<String> {
    if !consent {
        return None;
    }
    let line = warning(input?)?;
    Some(serde_json::json!({ "systemMessage": line }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nothing_running_or_a_missing_or_wrong_shaped_field_prints_nothing() {
        for input in [
            json!({}),
            json!({"background_tasks": [], "session_crons": []}),
            json!({"background_tasks": "x", "session_crons": {"a": 1}}),
            json!({"background_tasks": null, "session_crons": -4}),
            json!([1, 2]),
            json!("Stop"),
        ] {
            assert_eq!(warning(&input), None, "{input}");
        }
    }

    #[test]
    fn the_line_has_a_fixed_template_with_counts_and_fixed_kinds() {
        let input = json!({
            "background_tasks": [{"type": "shell"}, {"kind": "monitor"}, {"type": "Shell"}, {"type": "agent"}],
            "session_crons": [{}, {}]
        });
        assert_eq!(
            warning(&input).unwrap(),
            "klatch: this session stopped with 4 background task(s) (shell 2, monitor 1, agent 1) and 2 cron(s) still running. Stop them, or start one wait only (klatch wait)."
        );
    }

    #[test]
    fn unknown_missing_or_odd_kinds_and_bare_numbers_count_as_other() {
        let input = json!({"background_tasks": [{"type": "weird"}, {}, 7, {"type": 3}], "session_crons": 1});
        assert!(
            warning(&input)
                .unwrap()
                .contains("4 background task(s) (other 4) and 1 cron(s)")
        );
        let n = json!({"background_tasks": 2});
        assert!(
            warning(&n)
                .unwrap()
                .contains("2 background task(s) (other 2) and 0 cron(s)")
        );
    }

    #[test]
    fn only_crons_still_say_so_and_no_task_text_is_echoed() {
        let input = json!({
            "background_tasks": [{"type": "shell", "command": "SECRET-CMD", "description": "SECRET-DESC", "id": "SECRET-ID"}],
            "session_crons": [{"schedule": "SECRET-CRON", "prompt": "SECRET-PROMPT"}, {"type": "evil\"} injected"}]
        });
        let out = output(Some(&input), true).unwrap();
        assert!(
            !out.contains("SECRET") && !out.contains("injected"),
            "{out}"
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert!(v["systemMessage"].as_str().unwrap().contains("shell 1"));
        let crons = json!({"session_crons": [1]});
        assert!(
            warning(&crons)
                .unwrap()
                .contains("0 background task(s) (none) and 1 cron(s)")
        );
    }

    #[test]
    fn without_the_consent_or_the_input_nothing_is_printed() {
        let input = json!({"background_tasks": [{"type": "shell"}]});
        assert_eq!(output(Some(&input), false), None);
        assert_eq!(output(None, true), None);
    }
}
