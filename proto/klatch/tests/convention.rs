//! `klatch convention` prints the text a parent pastes into a subagent's brief (#491): the convention, with this
//! binary's own path and a name for the child, so the child never has to guess either.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    // No store, no home and no name: the command must work for a parent that has not joined anything.
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout.clone()).unwrap()
}

#[test]
fn it_prints_the_convention_with_the_path_of_the_binary_that_ran() {
    let t = text(&run(&["convention"]));
    let bin = env!("CARGO_BIN_EXE_klatch");
    assert!(t.contains(klatch::domain::CONVENTION_MARKER), "{t}");
    assert!(t.contains(&format!("{bin} --discover wait lobby")), "{t}");
}

#[test]
fn a_name_goes_in_front_of_every_command_so_a_child_does_not_share_its_parents() {
    let t = text(&run(&["convention", "--name", "fixer-1"]));
    let bin = env!("CARGO_BIN_EXE_klatch");
    assert!(
        t.contains(&format!("KLATCH_NAME=fixer-1 {bin} --discover join lobby")),
        "{t}"
    );
    assert!(
        t.contains(&format!("KLATCH_NAME=fixer-1 {bin} --discover post lobby")),
        "{t}"
    );
}

#[test]
fn without_a_name_it_tells_the_child_to_pick_a_unique_one() {
    let t = text(&run(&["convention"]));
    assert!(t.contains("KLATCH_NAME=<your-unique-name>"), "{t}");
}

#[test]
fn a_name_that_could_carry_a_shell_command_is_refused() {
    let o = run(&["convention", "--name", "x; rm -rf ~"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("name"));
}

#[test]
fn it_says_to_use_this_binary_and_not_a_fresh_build() {
    let t = text(&run(&["convention"]));
    assert!(t.contains("not a freshly built"), "{t}");
}

/// #1184: the commands the text teaches are real: each exists with the shape the text shows (a drift test).
#[test]
fn the_commands_the_text_teaches_exist_with_the_shape_it_shows() {
    let bin = env!("CARGO_BIN_EXE_klatch");
    let t = text(&run(&["convention"]));
    for (cmd, shape) in [
        ("dm", "<MEMBER> [BODY]..."),
        ("ask", "<ROOM> [TEXT]..."),
        ("answer", "<ROOM> <NUMBER> [TEXT]..."),
        ("find", "<ROOM> <TEXT>..."),
    ] {
        assert!(
            t.contains(&format!("{bin} --discover {cmd} ")),
            "{cmd}: {t}"
        );
        let h = text(&run(&[cmd, "--help"]));
        assert!(
            h.contains(shape),
            "`klatch {cmd} --help` has no {shape}: {h}"
        );
    }
}
