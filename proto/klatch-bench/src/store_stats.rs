//! `klatch-bench store-stats`: the counts-only usage report of a benchmark RUN store, without the admin passphrase
//! (#1433). It is the report `klatch admin stats --json` prints (#1377), built by the same library function
//! (`SqliteStore::read_usage_stats`) over the same store file, so the two cannot drift; it prints counts, timestamps and
//! room and member names, never a message body, ask text, claim detail, label or payload.
//!
//! It lives in the bench crate and not in the shipped binary on purpose: the admin gate (passphrase at the person's own
//! terminal, audit row) is the only way the shipped `klatch` shows this metadata, and this tool must not become a way
//! round it. So it refuses a store that could be a real one: the default live store (`$HOME/.klatch`) or anything
//! under it, a key directory that is not mode 700 (the way `KLATCH_KEY_DIR` is held), and a store with no key file in
//! that directory. It opens the file read-only (nothing created, no migration), and it is an offline harness tool.

use klatch::domain::{StatsOptions, start_of_day};
use klatch::keys::{KEY_LEN, Key};
use std::path::{Path, PathBuf};

/// What the command line asked for.
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub home: Option<PathBuf>,
    pub key_dir: Option<PathBuf>,
    pub json: bool,
    pub room: Option<String>,
    pub since: Option<String>,
    pub top: Option<usize>,
}

/// Parse the arguments after `store-stats`.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args::default();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--home" => a.home = Some(PathBuf::from(value("--home")?)),
            "--key-dir" => a.key_dir = Some(PathBuf::from(value("--key-dir")?)),
            "--json" => a.json = true,
            "--room" => a.room = Some(value("--room")?),
            "--since" => a.since = Some(value("--since")?),
            "--top" => {
                a.top = Some(
                    value("--top")?
                        .parse()
                        .map_err(|_| "--top takes a number".to_string())?,
                )
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

/// The default live store of the person running this: `$HOME/.klatch`.
pub fn live_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".klatch"))
}

/// Make the report for `args`. `live` is the live store directory to refuse (the caller passes `live_home()`).
pub fn report(args: &Args, live: Option<&Path>, now: i64) -> Result<String, String> {
    let home = args.home.as_ref().ok_or("--home DIR is required")?;
    let key_dir = args.key_dir.as_ref().ok_or("--key-dir DIR is required")?;
    let home = std::fs::canonicalize(home).map_err(|e| format!("cannot read --home: {e}"))?;
    refuse_live(&home, live)?;
    check_key_dir_mode(key_dir)?;
    let key = read_key(key_dir)?;
    let db = home.join("rooms.db");
    if !db.is_file() {
        return Err("no store (rooms.db) in --home".into());
    }
    let since = match args.since.as_deref() {
        Some(d) => Some(start_of_day(d).ok_or("--since takes a day as YYYY-MM-DD (UTC)")?),
        None => None,
    };
    let opts = StatsOptions {
        room: args.room.clone(),
        since,
        top: args.top.unwrap_or(10),
        now,
    };
    let stats = klatch::store::SqliteStore::read_usage_stats(&db, &key, &opts)
        .map_err(|e| format!("cannot read the store: {e}"))?;
    Ok(if args.json {
        format!("{}\n", stats.to_json())
    } else {
        stats.to_text()
    })
}

/// Refuse the live store, anything under it, and any directory that contains it.
fn refuse_live(home: &Path, live: Option<&Path>) -> Result<(), String> {
    let Some(live) = live else { return Ok(()) };
    // a live directory that does not exist cannot be the target; compare as written then
    let live = std::fs::canonicalize(live).unwrap_or_else(|_| live.to_path_buf());
    if home.starts_with(&live) || live.starts_with(home) {
        return Err(
            "refused: that is the live klatch store (or contains it); this tool reads benchmark run stores only".into(),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn check_key_dir_mode(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let md = std::fs::metadata(dir).map_err(|e| format!("cannot read --key-dir: {e}"))?;
    if !md.is_dir() {
        return Err("--key-dir is not a directory".into());
    }
    let mode = md.permissions().mode() & 0o777;
    if mode != 0o700 {
        return Err(format!(
            "refused: --key-dir is mode {mode:o}, it must be 700 (the way a key directory is held)"
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_key_dir_mode(_dir: &Path) -> Result<(), String> {
    Err("refused: key directory modes cannot be checked on this platform".into())
}

fn read_key(dir: &Path) -> Result<Key, String> {
    let file = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read --key-dir: {e}"))?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| crate::mp::is_main_key_file(&n.to_string_lossy()))
        })
        .ok_or("no store key file in --key-dir")?;
    let bytes = std::fs::read(&file).map_err(|e| format!("cannot read the key file: {e}"))?;
    let arr: [u8; KEY_LEN] = bytes
        .try_into()
        .map_err(|_| "the key file is not a store key".to_string())?;
    Ok(Key::from_bytes(arr))
}

/// The command: print the report, or say why not. The exit code.
pub fn main(args: impl Iterator<Item = String>) -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match parse(args).and_then(|a| report(&a, live_home().as_deref(), now)) {
        Ok(text) => {
            use std::io::Write;
            // a closed pipe is the reader's choice, not an error (check-sigpipe.sh)
            let _ = std::io::stdout().write_all(text.as_bytes());
            0
        }
        Err(e) => {
            eprintln!("klatch-bench store-stats: {e}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    const T0: i64 = 1_795_392_000;
    const SENTINELS: [&str; 6] = [
        "SENTINELMSGBODY",
        "SENTINELASKTEXT",
        "SENTINELCLAIMREASON",
        "SENTINELCLAIMPATH",
        "SENTINELREFVALUE",
        "SENTINELANSWER",
    ];

    /// A world with known content: messages, asks (one answered, one open), claims (granted, refused, released) and a ref.
    fn seeded() -> World {
        let w = World::new(&crate::find_binary().expect("klatch binary"));
        w.set_clock(T0);
        for who in ["alice", "bob", "carol"] {
            w.ok(who, &["join", "lobby"]).unwrap();
        }
        w.ok("alice", &["post", "lobby", "SENTINELMSGBODY one"])
            .unwrap();
        w.advance(5);
        let said = w
            .ok(
                "alice",
                &[
                    "ask",
                    "lobby",
                    "SENTINELASKTEXT",
                    "--ref",
                    "doc:SENTINELREFVALUE",
                ],
            )
            .unwrap();
        let n = said
            .split("asked #")
            .nth(1)
            .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
            .expect("ask number")
            .to_string();
        w.ok("alice", &["ask", "lobby", "an open one SENTINELASKTEXT"])
            .unwrap();
        w.advance(20);
        w.ok("bob", &["answer", "lobby", &n, "SENTINELANSWER"])
            .unwrap();
        w.ok(
            "alice",
            &[
                "claim",
                "lobby",
                "src/SENTINELCLAIMPATH.rs",
                "--reason",
                "SENTINELCLAIMREASON",
            ],
        )
        .unwrap();
        w.refused("bob", &["claim", "lobby", "src/SENTINELCLAIMPATH.rs"])
            .unwrap();
        w.advance(10);
        w.ok("alice", &["release", "lobby", "src/SENTINELCLAIMPATH.rs"])
            .unwrap();
        w.ok("bob", &["claim", "lobby", "src/SENTINELCLAIMPATH.rs"])
            .unwrap();
        w
    }

    fn key_dir(w: &World) -> PathBuf {
        w.home.join("keys")
    }

    fn args(w: &World) -> Args {
        Args {
            home: Some(w.home.clone()),
            key_dir: Some(key_dir(w)),
            json: true,
            ..Args::default()
        }
    }

    fn json_without(text: &str, drop: &[&str]) -> serde_json::Value {
        let mut v: serde_json::Value = serde_json::from_str(text.trim()).expect("one JSON object");
        for k in drop {
            v.as_object_mut().unwrap().remove(*k);
        }
        v
    }

    #[test]
    fn the_report_equals_klatch_admin_stats_json_for_the_same_store() {
        let w = seeded();
        let admin = w.admin(&["stats", "--json"], "");
        assert!(admin.ok, "admin stats failed: {}", admin.stdout);
        let line = admin
            .stdout
            .lines()
            .find(|l| l.starts_with('{'))
            .expect("admin printed a JSON line");
        let at = json_without(line, &[])["at"].as_i64().unwrap();
        let ours = report(&args(&w), None, at).unwrap();
        // the admin act writes its own audit row after it reports, so a later read counts one more `stats`
        let mut a = json_without(line, &[]);
        let mut o = json_without(&ours, &[]);
        let strip = |v: &mut serde_json::Value| {
            v["audit"]
                .as_array_mut()
                .unwrap()
                .retain(|e| e["name"] != "stats");
        };
        strip(&mut a);
        strip(&mut o);
        assert_eq!(a, o);
        // and it is a real report: the seeded content is in it
        assert_eq!(o["asks"]["created"], 2);
        assert_eq!(o["asks"]["answered"], 1);
        assert_eq!(o["claims"]["history"]["conflicts_handled"], 1);
    }

    #[test]
    fn the_text_form_prints_a_report_of_the_same_store() {
        let w = seeded();
        let mut a = args(&w);
        a.json = false;
        let text = report(&a, None, T0 + 100).unwrap();
        assert!(text.contains("asks"), "text report: {text}");
    }

    #[test]
    fn no_sentinel_text_reaches_the_output_for_any_option_combination() {
        let w = seeded();
        let rooms = [None, Some("lobby".to_string())];
        let sinces = [None, Some("2026-11-23".to_string())];
        let tops = [None, Some(1), Some(50)];
        let mut combos = Vec::new();
        for json in [true, false] {
            for room in &rooms {
                for since in &sinces {
                    combos.extend(
                        tops.iter()
                            .map(|top| (json, room.clone(), since.clone(), *top)),
                    );
                }
            }
        }
        assert_eq!(combos.len(), 24);
        for (json, room, since, top) in combos {
            let a = Args {
                json,
                room,
                since,
                top,
                ..args(&w)
            };
            let out = report(&a, None, T0 + 100).unwrap();
            assert!(out.len() > 20);
            for s in SENTINELS {
                assert!(!out.contains(s), "{s} leaked with {a:?}");
            }
        }
    }

    #[test]
    fn the_live_store_and_anything_under_or_over_it_is_refused() {
        let w = seeded();
        let live_parent = w.home.parent().unwrap().to_path_buf();
        let live = w.home.clone();
        // the very directory, treated as the live store
        let e = report(&args(&w), Some(&live), T0).unwrap_err();
        assert!(e.contains("live klatch store"), "{e}");
        // a directory under it
        let under = live.join("keys");
        let mut a = args(&w);
        a.home = Some(under);
        let e = report(&a, Some(&live), T0).unwrap_err();
        assert!(e.contains("live klatch store"), "{e}");
        // a directory that contains it
        let mut a = args(&w);
        a.home = Some(live_parent);
        let e = report(&a, Some(&live), T0).unwrap_err();
        assert!(e.contains("live klatch store"), "{e}");
        // a different directory is fine
        let other = crate::test_dir::TestDir::new("1433-live");
        report(&args(&w), Some(&other), T0).unwrap();
    }

    #[test]
    fn a_default_live_path_under_home_is_refused_even_through_a_symlink() {
        let w = seeded();
        let tmp = crate::test_dir::TestDir::new("1433-sym");
        let link = tmp.join("link");
        std::fs::create_dir_all(&tmp).unwrap();
        std::os::unix::fs::symlink(&w.home, &link).unwrap();
        let mut a = args(&w);
        a.home = Some(link);
        let e = report(&a, Some(&w.home), T0).unwrap_err();
        assert!(e.contains("live klatch store"), "{e}");
    }

    #[test]
    fn a_loose_key_directory_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let w = seeded();
        let dir = key_dir(&w);
        for mode in [0o755, 0o750, 0o770, 0o500] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
            let e = report(&args(&w), None, T0).unwrap_err();
            assert!(e.contains("must be 700"), "mode {mode:o}: {e}");
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        report(&args(&w), None, T0).unwrap();
    }

    #[test]
    fn a_missing_store_is_an_error_and_nothing_is_created() {
        let w = seeded();
        let empty = crate::test_dir::TestDir::new("1433-empty");
        std::fs::create_dir_all(&empty).unwrap();
        let mut a = args(&w);
        a.home = Some(empty.to_path_buf());
        let e = report(&a, None, T0).unwrap_err();
        assert!(e.contains("no store"), "{e}");
        assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);
    }

    #[test]
    fn a_wrong_key_is_an_error_not_a_report() {
        let w = seeded();
        let keys = crate::test_dir::TestDir::new("1433-wrongkey");
        std::fs::create_dir_all(&keys).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(keys.join("store-x.key"), [7u8; 32]).unwrap();
        let mut a = args(&w);
        a.key_dir = Some(keys.to_path_buf());
        assert!(report(&a, None, T0).is_err());
    }

    #[test]
    fn arguments_parse_and_unknown_ones_are_refused() {
        let a = parse(
            ["--home", "h", "--key-dir", "k", "--json", "--top", "3"]
                .iter()
                .map(|s| s.to_string()),
        )
        .unwrap();
        assert!(a.json && a.top == Some(3));
        assert!(parse(["--passphrase".to_string()].into_iter()).is_err());
    }
}
