//! What `klatch wait` listens for in each room (#219): a body prefix filter and a sender filter, set for every room
//! by `--only` and `--from` or for one room by `ROOM:only=PREFIX` and `ROOM:from=MEMBER`.

use super::{Message, valid_name};

const FORM: &str = "write a room alone (`lobby`) or with one filter (`lobby:only=seeking:` or `lobby:from=bob`; name the room again for another)";

const DM_FORM: &str =
    "write `dm:NAME` for your direct messages with that member (`dm:bob`, or `dm:bob:from=bob`)";

/// Rewrite each `dm:NAME` (optionally `dm:NAME:only=PREFIX` or `dm:NAME:from=MEMBER`) to the caller's dm room with
/// NAME, `dm.<a>.<b>` (#1186). `dm:only=...` and `dm:from=...` stay a room named `dm` with a filter. The error
/// names only the argument and a fixed form, never message text.
pub fn resolve_dm_targets(args: &[String], me: &str) -> Result<Vec<String>, String> {
    args.iter()
        .map(|arg| Ok(dm_room_target(arg, me)?.unwrap_or_else(|| arg.clone())))
        .collect()
}

/// The room argument for one `dm:NAME[:spec]` argument, `None` for any other argument.
fn dm_room_target(arg: &str, me: &str) -> Result<Option<String>, String> {
    let Some(rest) = arg.strip_prefix("dm:") else {
        return Ok(None);
    };
    if rest.starts_with("only=") || rest.starts_with("from=") {
        return Ok(None);
    }
    let (name, spec) = match rest.split_once(':') {
        Some((name, spec)) => (name, Some(spec)),
        None => (rest, None),
    };
    if !valid_name(name) {
        return Err(format!(
            "`{}` is not a member name; {DM_FORM}",
            super::shown(arg)
        ));
    }
    if name == me {
        return Err(format!("`{}` is yourself; {DM_FORM}", super::shown(arg)));
    }
    let room = super::dm_room(me, name);
    Ok(Some(match spec {
        Some(spec) => format!("{room}:{spec}"),
        None => room,
    }))
}

/// What one room's wait accepts. An empty list accepts everything; a non-empty one accepts any of its entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub only: Vec<String>,
    pub from: Vec<String>,
}

impl Filter {
    pub fn is_open(&self) -> bool {
        self.only.is_empty() && self.from.is_empty()
    }

    pub fn accepts(&self, m: &Message) -> bool {
        let prefix_ok = self.only.is_empty() || self.only.iter().any(|p| m.body.starts_with(p));
        let sender_ok = self.from.is_empty() || self.from.contains(&m.sender);
        prefix_ok && sender_ok
    }
}

/// The rooms of a wait in the order given, each once, with its filter. A room named with a filter (even twice, even
/// alongside the bare name) uses only the filters written on it; a bare room uses `global`.
pub fn room_filters(args: &[String], global: &Filter) -> Result<Vec<(String, Filter)>, String> {
    let mut rooms: Vec<(String, Option<Filter>)> = Vec::new();
    for arg in args {
        let (room, spec) = match arg.split_once(':') {
            Some((room, spec)) => (room, Some(spec)),
            None => (arg.as_str(), None),
        };
        if !valid_name(room) {
            return Err(format!("`{}` is not a room; {FORM}", super::shown(arg)));
        }
        let at = match rooms.iter().position(|(r, _)| r == room) {
            Some(at) => at,
            None => {
                rooms.push((room.to_string(), None));
                rooms.len() - 1
            }
        };
        if let Some(spec) = spec {
            let f = rooms[at].1.get_or_insert_with(Filter::default);
            add_spec(f, spec).map_err(|e| format!("`{}`: {e}; {FORM}", super::shown(arg)))?;
        }
    }
    Ok(rooms
        .into_iter()
        .map(|(room, f)| (room, f.unwrap_or_else(|| global.clone())))
        .collect())
}

fn add_spec(f: &mut Filter, spec: &str) -> Result<(), String> {
    match spec.split_once('=') {
        Some(("only", p)) if !p.is_empty() => f.only.push(p.to_string()),
        Some(("from", m)) if valid_name(m) => f.from.push(m.to_string()),
        _ => return Err("the filter must be `only=PREFIX` or `from=MEMBER`".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// #1776: a name the operator typed is echoed in the refusal; a terminal escape, a bidi override, a zero-width
    /// character or a line break in it must not reach the terminal raw.
    fn assert_escaped(e: &str, what: &str) {
        assert!(e.contains("\\u{1b}"), "{what}: {e}");
        assert!(
            !e.chars().any(|c| c.is_control()) && !e.contains(['\u{202e}', '\u{200b}']),
            "{what}: {e:?}"
        );
    }

    #[test]
    fn a_refused_argument_is_echoed_escaped() {
        let h = "\u{1b}[2J\u{202e}\u{200b}\n";
        for a in [format!("{h}x"), format!("lobby:{h}x")] {
            let e = room_filters(&[a.clone()], &Filter::default()).unwrap_err();
            assert_escaped(&e, &a);
        }
        for a in [format!("dm:{h}x"), format!("dm:{h}x:only=a")] {
            let e = resolve_dm_targets(&[a.clone()], "me").unwrap_err();
            assert_escaped(&e, &a);
        }
        let e = resolve_dm_targets(&["dm:me".to_string()], "me").unwrap_err();
        assert!(e.contains("`dm:me` is yourself"), "{e}");
    }

    #[test]
    fn a_bare_room_takes_the_global_filter_and_a_filtered_room_its_own() {
        let g = Filter {
            only: vec!["seeking:".into()],
            from: vec![],
        };
        let r = room_filters(&args(&["dm", "lobby:only=closing:"]), &g).unwrap();
        assert_eq!(r[0], ("dm".to_string(), g));
        assert_eq!(r[1].1.only, vec!["closing:".to_string()]);
    }

    #[test]
    fn a_room_named_twice_merges_its_filters_and_keeps_its_place() {
        let r = room_filters(
            &args(&["a", "b:only=x", "a", "b:only=y"]),
            &Filter::default(),
        )
        .unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].1.only, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn a_prefix_may_contain_a_colon_and_a_bad_form_is_refused() {
        let r = room_filters(&args(&["r:only=a:b:"]), &Filter::default()).unwrap();
        assert_eq!(r[0].1.only, vec!["a:b:".to_string()]);
        for bad in [
            "r:only=",
            "r:nope=1",
            "r:from=a b",
            "r:",
            ":only=x",
            "bad room",
        ] {
            assert!(
                room_filters(&args(&[bad]), &Filter::default()).is_err(),
                "{bad}"
            );
        }
    }
}
