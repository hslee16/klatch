The gate's leaked-sleep check counts only its own process group (#1101, #1127)

The check added in #1101 counted every `/bin/sleep 3600` on the machine, so a gate failed whenever another worktree, still on a base from before the watchdog fix, ran its own team tests at the same time. It now counts only sleeps in the gate's own process group, which an orphan keeps. The environment of a system binary such as `sleep` cannot be read on macOS, so a marker variable was not an option. A leak from a test that starts its own process group is covered by that test's own check (`test-team.sh`).
