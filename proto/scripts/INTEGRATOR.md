# The integrator process (#866, epic #863)

`proto/scripts/integrator` merges the PRs that members queue with `ready: #N`. It is the process behind the `integrator` role: it runs OUTSIDE the klatch binary, on its own machine, with its OWN `gh` and `git` login. The klatch binary makes no network call and holds no credential; this process reaches klatch only through the `klatch` command line (`wait`, `board`, `post`, `claim`, `release`). It replaces the cron loop around `pr-sweep` once the retirement ticket (#868) lands; until then both exist and only one of them should merge at a time.

## What the human does

1. **Grant the role**, at a terminal (an agent session cannot): `klatch admin integrator <member>` (`--off` removes it). The member is an ordinary member in every other way. Without the role the `merge:main` claim is refused with the rule `integrator`, and the integrator exits with code 2 and says so.
2. **Give the member an identity** the way every helper has one: a worker credential the human issues (`klatch worker issue`, see `proto/scripts/team`) and the environment that points at it (`KLATCH_IDENTITY_FILE`, `KLATCH_HOME`, `KLATCH_KEY_DIR`), or `KLATCH_NAME` on a machine where that is how members are named. The integrator reads none of it; the `klatch` command does.
3. **Prepare the machine** (never put a credential in the repository):
   - `gh auth login` as an account that can push to the PR branches and that has **admin** on the repository: `gh webhook forward` creates a temporary repository webhook through the REST API and removes it on exit, and that needs repo admin. Install the extension once: `gh extension install cli/gh-webhook`.
   - A DEDICATED clone of the repository, with a git identity (`git config user.name`, `user.email`) and push access to the PR branches (an SSH key or the `gh` credential helper). The integrator switches branches in it (`integrator/N`), so do not use a checkout you work in. It must be clean.
   - `git` 2.38 or newer (`git merge-tree --write-tree`), `python3`, `perl`, and whatever the project gate needs (the Rust toolchain, `cargo`).
4. **Check the setup with a dry run** (the first live check, before the first real merge):

   ```
   INTEGRATOR_REPO=/path/to/dedicated/clone proto/scripts/integrator --dry-run
   ```

   Run it as the integrator member (the same environment as the real run). It reads and prints; it posts nothing to klatch, does not take `merge:main` (it only reads who holds it), pushes nothing, makes no branch, merges nothing, runs no gate, does not start the forwarder and makes no write call to `gh`. Read the `setup:` lines first: each is `ok` or `PROBLEM <name>: <next step>` (`no-integrator-role`, `no-gh-login`, `no-admin`, `unreadable`, `no-webhook-extension`, `dirty-clone`, `wrong-branch`, `no-gate`, `no-board`). Fix every problem and run it again until none is left (the exit code is 1 while one is). Then read the per PR lines: they are the words a real run would post, in queue order, and a `dropped:` line says why a PR would be dropped. The last line is always `dry run: nothing was changed.` Limits: klatch shows no role, so the role line only says no refusal is on record (the first real run proves it, #1090); each PR is checked against main as it is now. Then start it for real.
5. **Start it**, in its own terminal or a service of that machine:

   ```
   INTEGRATOR_REPO=/path/to/dedicated/clone proto/scripts/integrator
   ```

   It runs until a signal (INT, TERM, HUP): it stops the forwarder, releases `merge:main` and exits 0. `--once` drains the queue and exits (for a hand run). Stop it with TERM or INT, never `kill -9` first: a forwarder that is killed hard cannot remove its temporary webhook, and one is then left on the repository (remove it in the repository settings or with `gh api repos/OWNER/REPO/hooks`).

Settings, all optional (environment): `INTEGRATOR_GH_REPO` (default `hslee16/klatch`), `INTEGRATOR_ROOM` (`lobby`), `INTEGRATOR_KLATCH` (the `klatch` command), `INTEGRATOR_GATE` (a command to run instead of `proto/scripts/gate`, run from the clone's root), `INTEGRATOR_LEASE_SECS` (600; klatch caps `merge:main` at 1800), `INTEGRATOR_RECONCILE_SECS` (300, at most 300), `INTEGRATOR_PARK_MINUTES` (25; klatch stretches `merge:main` for at most 30 minutes after a park), `INTEGRATOR_CHECK_WAIT_SECS` (5400). The others are test knobs and are named in the header of the script.

Exit codes: 0 stopped by a signal or `--once` done; 1 usage or setup; 2 refused the claim (no integrator role); 3 `merge:main` is held by another member, or was lost; 4 the event source could not be kept running; 5 halted after a merge made with `--merge-without-update` because main was red (and could not be put right) or could not be verified.

## What it does

- **Start**: takes `merge:main` once and releases it, to prove the role (a refusal ends the start with exit 2 and starts nothing else). Then it starts `gh webhook forward --repo <repo> --events pull_request,check_suite,workflow_run` as a child process and reads the open PR list.
- **Waiting**: with an empty queue it holds no claim and sits in `klatch wait <room> --only ready:`; klatch wakes it. It does not poll GitHub for work. Every `INTEGRATOR_RECONCILE_SECS` it reads the open PR list and the `mergeable_state` of each open PR again (events are lost while the forwarder is down).
- **The queue** is the board's queue section (item for #864): the entries shown as waiting, integrating or checking, in order. Only digits and a state word are read from the board text.
- **Per PR**:
  1. conflict pre-check with `git merge-tree` against `origin/main`; a conflict outside the docs files posts `dropped: #N conflict` and nothing is touched;
  2. `integrating: #N`; merge `origin/main` into the PR branch on `integrator/N` (never a rebase; docs conflicts keep both sides, with the same resolver as `pr-sweep --resolve-docs`); a PR that already contains main skips this step;
  3. `parked: running the gate for #N, back by 25m`, the project gate (`proto/scripts/gate`), a failure posts `dropped: #N checks-failed`;
  4. a plain push to the PR's own branch (never `--force`), then `checking: #N`;
  5. `parked: waiting for checks on #N, back by 25m`; wait for the checks of the CURRENT head sha: a check event wakes it, and the check runs of that head are read (all completed and success, skipped or neutral) before anything is merged;
  6. `gh pr merge N --merge --match-head-commit <sha>`, a read that says it is merged, `merged: #N`;
  7. after the merge it reads `mergeable_state` of every open PR again (GitHub sends no event when a PR starts to conflict) and drops a queued PR that now has a code conflict.
- **Dropped reasons** (a fixed list; the board shows the first three by name and any other as `other reason`): `conflict`, `checks-failed` (the gate, or a failed or cancelled check suite on the current head), `closed`, `timeout` (checks not green in `INTEGRATOR_CHECK_WAIT_SECS`), `head-changed` (someone pushed to the branch while it waited), `error` (a failed read, fetch, push or merge, a branch of another repository, a branch name that is not a plain name, a dirty clone).
- **Events** are data. The forwarder's payloads go through a normalizer that keeps a number, a state, merged, a head sha, a status and a conclusion, each checked against a fixed shape; titles, bodies, branch names and logins are never kept. A cancelled check counts as a failure only for the current head sha (it is only looked up for that sha, so a superseded run is ignored). A check suite on main with no PR is written to a side log (for #867) and never changes the queue.
- **Words** posted into klatch are fixed templates made of an integer and a word from the list above (`integrating: #N`, `checking: #N`, `merged: #N`, `dropped: #N <reason>`, `parked: <fixed text> #N, back by 25m`), never a title, a body or a branch name.

## Merge without updating the branch (#867, off by default)

`proto/scripts/integrator --merge-without-update [--verify-gate] [--verify-bench]` is a different merge policy. It is OFF unless the human passes the flag, and the flow above stays the default and is unchanged. **Using the flag replaces the up-to-date rule for the PRs it applies to: it is the human's decision to turn it on.** The old rule costs a re-merge of main, a gate and a round of checks per PR per merge; the owner's pick (2026-11-18) is to merge a PR that merges cleanly into current main and to check main afterwards instead.

- **Which PRs.** Only a PR whose `git merge-tree` against current `origin/main` reports no conflict at all. For it the integrator posts `integrating: #N`, does not merge main into the branch, runs no gate on it and pushes nothing, posts `checking: #N`, waits for the check runs of the PR's CURRENT head sha to be green, checks the merge-tree against main again (main may have moved: a conflict now posts `dropped: #N conflict`), and merges with `gh pr merge --match-head-commit <that head>`. A code conflict still posts `dropped: #N conflict`; a docs-only conflict is not clean, so that PR takes the default update flow.
- **Verifying main.** After the merge it reads the merge commit sha from the PR and waits for the `check_suite` and `workflow_run` events of that sha (they arrive with no PR attached and are already routed to the side log) and reads the check runs of that sha. All completed and green: nothing is posted. A failure (failure, timed out, startup failure, action required) is red. A cancelled run is not a failure. `INTEGRATOR_VERIFY_WAIT_SECS` bounds the wait (default the check wait).
- **`--verify-gate`** also runs the project gate on main at the merge commit (it takes minutes; a `parked:` word is posted first). **`--verify-bench`** also runs the bench (`INTEGRATOR_BENCH`, default `cargo run -q --release -p klatch-bench` from `proto/`). Either one failing counts as red. Both need `--merge-without-update`.
- **A red main attributed to the merge it just made** (the same sha; a red check on another sha is ignored, and a merge it did not make in this run is never reverted) is reverted, never by a push to main: branch `revert-N` from `origin/main`, `git revert -m 1` of the merge commit with a fixed commit message, a plain push of that branch, a PR with a fixed title (`Revert PR #N (main went red)`) and a fixed body, a wait for ITS checks on its current head, the same `--match-head-commit` merge, then main is verified again. On success it posts `reverted: #N` and goes on. No text of the reverted PR (title, body, branch) is used.
- **Halt.** If any revert step fails (a conflict, a push, the PR, red checks, the merge), or main is still red after the revert, it posts `halted: main red after #N`; if main could not be verified (no answer in time, only a cancelled run, no event source, an unreadable merge sha) it posts `halted: main not verified after #N`. Then it releases `merge:main`, merges nothing more and exits 5. It never reverts twice. The human looks at main and at the `revert-N` branch and PR, then starts the integrator again.
- **Words.** `reverted: #N` and `halted: main red after #N` / `halted: main not verified after #N` are fixed templates of an integer. Klatch does not read them (they are plain lobby lines; the board still shows the PR as merged); they are the lobby line the owner asked for. Showing them on the board is ticketed (#1094).
- **The dry run** (`--dry-run --merge-without-update`) describes this plan: which queued PRs would skip the update, the words, the verification and the revert path. It still changes nothing.
- **Limits.** The checks of the PR head are the PR alone, not the PR combined with the main of the moment: that is the policy change, and the verification above is what takes its place. Verification waits holding `merge:main`, so the next PR waits for it. The revert is of the merge commit this run made; a later merge on top of it by someone else may make the revert conflict, which halts.

## The claim and the lease

`merge:main` is taken when there is work, renewed at each step and at each reconcile while the forwarder is alive, and released when the queue is empty and on exit. The two long steps (the gate and the wait for checks) are declared with `parked:`, which stretches the claim up to the 30 minute park bound; the stretch never goes beyond it. A dead forwarder gets no renewal: the process restarts it (three quick failures in a row end the process with exit 4 and the claim released), so a forwarder that dies cannot leave the claim held, and a process that goes silent lapses at the lease.

## Limits

- One forwarder per repository has been tested; concurrent forwarders have not.
- The process takes PRs from branches of the same repository only (a branch of a fork is dropped with `error`). It runs the project gate on what a PR contains, so it trusts the members that may push to the repository, as the sweep did.
- A process that is stopped mid PR leaves the board entry as integrating or checking; the next start sees it and does the PR again from the start.
- Not here: retiring `pr-sweep`, `pr-ledger` and the cron loop (#868), and any klatch-server side work (#1056).
- Tests: `proto/scripts/test-integrator.sh` (part of the gate) runs the process against a fake `gh`, a fake `klatch`, a fake gate and temp git repositories, with webhook payload fixtures in `integrator-fixtures/` shaped after GitHub's documented payloads (they are not recordings of the live repository). It never touches GitHub, a real repository or a real klatch home. A bench scenario was not added: the bench drives klatch members, not `gh` or a git remote, so it cannot run this process with fakes.
