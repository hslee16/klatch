//! A `ps` found only on a hostile PATH is never run (#1568). One test in this file, so setting PATH cannot race another.

use klatch::harness::{Lookup, ProcessTable, PsTable};
use std::os::unix::fs::PermissionsExt;

#[allow(dead_code)]
#[path = "../src/store/test_dir.rs"]
mod test_dir;

#[test]
fn a_fake_ps_first_on_path_is_not_run_and_the_real_table_still_answers() {
    let dir = test_dir::TestDir::new("ps-fixed-path");
    let marker = dir.join("marker");
    let fake = dir.join("ps");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho ran >> '{}'\necho '  1 0 Fri Nov 6 03:04:05 2026 evil'\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old = std::env::var("PATH").unwrap_or_default();
    // SAFETY: the only test in this process, and no other thread reads the environment meanwhile
    unsafe { std::env::set_var("PATH", format!("{}:{old}", dir.display())) };

    let own = PsTable.lookup(std::process::id());
    let pinned = PsTable
        .pin()
        .expect("the real table pins a snapshot")
        .lookup(std::process::id());

    unsafe { std::env::set_var("PATH", old) };
    let ran = marker.exists();
    assert!(!ran, "the ps on PATH was run");
    assert!(matches!(own, Lookup::Found(_)), "{own:?}");
    assert!(matches!(pinned, Lookup::Found(_)), "{pinned:?}");
}
