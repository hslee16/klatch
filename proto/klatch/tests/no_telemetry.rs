//! R24 (#80): no telemetry, no third-party calls, no unverified remote installer. This is an audit by inspection: it
//! checks what the binary could do (its dependencies and its source), not what it did on one run, and it cannot prove a
//! negative about code it does not read. What it pins is that nothing in klatch opens a network connection or runs a
//! network program, and that the documents never tell anyone to pipe a downloaded script into a shell.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn rust_sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect()
}

#[test]
fn no_networking_crate_is_in_the_dependency_tree() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "tree",
            "-p",
            "klatch",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8(out.stdout).unwrap();
    let deps: Vec<&str> = tree
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    const NETWORK: &[&str] = &[
        "reqwest",
        "hyper",
        "hyper-util",
        "h2",
        "ureq",
        "curl",
        "curl-sys",
        "isahc",
        "attohttpc",
        "surf",
        "minreq",
        "tungstenite",
        "tokio-tungstenite",
        "tokio",
        "async-std",
        "smol",
        "mio",
        "socket2",
        "rustls",
        "native-tls",
        "openssl",
        "quinn",
        "hickory-resolver",
        "trust-dns-resolver",
        "reqwest-middleware",
    ];
    for n in NETWORK {
        assert!(
            !deps.contains(n),
            "a networking crate is in klatch's dependencies: {n}"
        );
    }
    assert!(deps.len() > 20, "the tree looks empty: {deps:?}");
}

/// Test-only modules that may run a program chosen at run time: they run the test binary again in a child process with
/// a private environment, or start a stand-in for the coding agent's process, so key, registry and home paths can be
/// tested without the real ones. Exact paths relative to the crate, never a suffix: a new file needs a new line here,
/// and must be declared `#[cfg(test)] mod name;` in its parent so it is never part of the shipped binary (#771).
const TEST_ONLY_SPAWNERS: &[&str] = &[
    "src/admin/command_tests.rs",
    "src/custody/survivor_tests.rs",
    "src/identity/survivor_tests.rs",
];

/// The text of the file that declares the module at `rel` (`src/a/b.rs` is declared in `src/a.rs` or `src/a/mod.rs`),
/// for the first of them that exists.
fn parent_text(rel: &str) -> Option<String> {
    let (dir, _) = rel.rsplit_once('/')?;
    [format!("{dir}.rs"), format!("{dir}/mod.rs")]
        .iter()
        .find_map(|p| std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(p)).ok())
}

/// Whether `text` declares `mod name;` directly under `#[cfg(test)]` (other attributes may sit between them).
fn declared_under_cfg_test(text: &str, name: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let want = format!("mod {name};");
    lines.iter().enumerate().any(|(i, l)| {
        *l == want
            && lines[..i]
                .iter()
                .rev()
                .take_while(|a| a.starts_with("#["))
                .any(|a| *a == "#[cfg(test)]")
    })
}

/// Whether the file at `rel` may run a program chosen at run time: it is listed in `exempt` by its exact path and
/// `parent_text` (the declaring file) declares it under `#[cfg(test)]`.
fn is_test_only_spawner(
    rel: &str,
    exempt: &[&str],
    parent_text: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let name = rel
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".rs"))
        .unwrap_or("");
    exempt.contains(&rel)
        && parent_text(rel).is_some_and(|text| declared_under_cfg_test(&text, name))
}

/// What is wrong with a `Command::new(...)` on this line of code, if anything: it runs a program that is not on the
/// list, or one chosen at run time outside the client and the test-only modules.
fn command_problem(rel: &str, code: &str, programs: &[&str]) -> Option<String> {
    let arg = &code[code.find("Command::new(")? + "Command::new(".len()..];
    match arg.strip_prefix('"').and_then(|a| a.split('"').next()) {
        Some(program) if programs.contains(&program) => None,
        Some(program) => Some(format!("runs {program}")),
        None if rel.ends_with("src/client.rs") => None,
        None if is_test_only_spawner(rel, TEST_ONLY_SPAWNERS, &parent_text) => None,
        None => Some("runs a program chosen at run time".to_string()),
    }
}

#[test]
fn the_source_opens_no_socket_and_runs_only_the_programs_it_is_known_to_run() {
    const SOCKETS: &[&str] = [
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "UnixStream",
        "UnixListener",
        "UnixDatagram",
    ]
    .as_slice();
    // Programs klatch starts by name: git (to find a repository), ps (the process table), secret-tool and security
    // (the keystores), codex (only `--version` and `queue --help`, in `klatch doctor`: no login, no model call), claude (only `--version`, in `klatch doctor` and before `klatch wake`, #482: no login, no model call), and sh only in a unit test that waits for a fake keystore script to be executable. None of them is asked to talk to a network. The Rust client starts the binary the caller names.
    const PROGRAMS: &[&str] = &[
        "git",
        "ps",
        "secret-tool",
        "/usr/bin/security",
        "codex",
        "claude",
        "sh",
    ];
    let mut problems = Vec::new();
    for (path, text) in rust_sources() {
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .display()
            .to_string();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let at = format!("{rel}:{}", n + 1);
            problems.extend(
                SOCKETS
                    .iter()
                    .filter(|s| code.contains(*s))
                    .map(|s| format!("{at}: uses {s}")),
            );
            problems.extend(command_problem(&rel, code, PROGRAMS).map(|p| format!("{at}: {p}")));
        }
    }
    assert!(
        problems.is_empty(),
        "outbound capability in the source:\n{}",
        problems.join("\n")
    );
}

#[test]
fn std_net_is_only_used_to_read_addresses() {
    for (path, text) in rust_sources() {
        for line in text.lines().filter(|l| l.contains("std::net")) {
            for item in [
                "TcpStream",
                "TcpListener",
                "UdpSocket",
                "ToSocketAddrs",
                "lookup_host",
            ] {
                assert!(!line.contains(item), "{}: {line}", path.display());
            }
        }
    }
}

#[test]
fn the_documents_never_tell_anyone_to_pipe_a_download_into_a_shell() {
    for doc in [
        "README.md",
        "UNINSTALL.md",
        "RELEASING.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "install/install.sh",
    ] {
        let text = std::fs::read_to_string(repo(doc)).unwrap();
        for (n, line) in text.lines().enumerate() {
            let piped = line.contains("| sh")
                || line.contains("| bash")
                || line.contains("|sh")
                || line.contains("|bash");
            let fetches = line.contains("curl") || line.contains("wget");
            assert!(
                !(piped && fetches),
                "{doc}:{}: pipes a download into a shell: {line}",
                n + 1
            );
        }
    }
}

#[test]
fn the_installer_talks_only_to_github() {
    let script = std::fs::read_to_string(repo("install/install.sh")).unwrap();
    for word in script.split(|c: char| c.is_whitespace() || "\"'()<>".contains(c)) {
        if let Some(rest) = word
            .strip_prefix("https://")
            .or_else(|| word.strip_prefix("http://"))
        {
            let host = rest.split(['/', ':']).next().unwrap();
            assert!(
                host == "github.com" || host == "api.github.com" || host.starts_with('$'),
                "install.sh mentions another host: {word}"
            );
            assert!(
                word.starts_with("https://"),
                "install.sh uses plain http: {word}"
            );
        }
    }
}

#[test]
fn a_test_module_is_exempt_only_by_exact_path_and_only_when_declared_under_cfg_test() {
    let declared = |_: &str| Some("#[cfg(test)]\nmod survivor_tests;\n".to_string());
    let exempt = ["src/a/survivor_tests.rs"];
    assert!(is_test_only_spawner(
        "src/a/survivor_tests.rs",
        &exempt,
        &declared
    ));
    // the same file name in another directory is not exempt, whatever its parent says
    assert!(!is_test_only_spawner(
        "src/b/survivor_tests.rs",
        &exempt,
        &declared
    ));
    assert!(!is_test_only_spawner(
        "src/a/b/survivor_tests.rs",
        &exempt,
        &declared
    ));
    assert!(!is_test_only_spawner(
        "src/a/survivor_tests.rs.bak",
        &exempt,
        &declared
    ));
    // listed, but the parent declares it without #[cfg(test)], with cfg(not(test)), or not at all
    let plain = |_: &str| Some("mod survivor_tests;\n".to_string());
    let not_test = |_: &str| Some("#[cfg(not(test))]\nmod survivor_tests;\n".to_string());
    let other_mod = |_: &str| Some("#[cfg(test)]\nmod other_tests;\n".to_string());
    let separated = |_: &str| Some("#[cfg(test)]\nfn f() {}\nmod survivor_tests;\n".to_string());
    let none = |_: &str| None;
    for parent in
        [&plain, &not_test, &other_mod, &separated] as [&dyn Fn(&str) -> Option<String>; 4]
    {
        assert!(!is_test_only_spawner(
            "src/a/survivor_tests.rs",
            &exempt,
            parent
        ));
    }
    assert!(!is_test_only_spawner(
        "src/a/survivor_tests.rs",
        &exempt,
        &none
    ));
    // another attribute between the cfg and the module is fine
    let attr =
        |_: &str| Some("#[cfg(test)]\n#[allow(dead_code)]\nmod survivor_tests;\n".to_string());
    assert!(is_test_only_spawner(
        "src/a/survivor_tests.rs",
        &exempt,
        &attr
    ));
}

#[test]
fn every_listed_test_only_spawner_exists_and_is_declared_under_cfg_test_in_the_real_source() {
    for rel in TEST_ONLY_SPAWNERS {
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(rel).is_file(),
            "{rel} is listed but does not exist: remove it from TEST_ONLY_SPAWNERS"
        );
        assert!(
            is_test_only_spawner(rel, TEST_ONLY_SPAWNERS, &parent_text),
            "{rel}: its parent does not declare it as #[cfg(test)] mod ...;"
        );
    }
}
