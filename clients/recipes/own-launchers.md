# Launchers klatch ships or documents: recipes you can check

This page is for someone who wants klatch under a launcher and has no desktop orchestrator, or has not run one yet. Every
launcher here is either a plain script, a Makefile, or a script in `proto/scripts`. None needs an account or a desktop app.
The third-party orchestrators (Claude Code agent teams, Orchestrator.inc, Claude Squad, Conductor, Agent Swarm, a general
framework) are **not run**: that stays ticket #414 and needs a person; see [ORCHESTRATORS.md](ORCHESTRATORS.md) for what is
only described. What is checked here is checked by `clients/recipes/test_own_launchers_doc.py`, and the checks are named
under each recipe so a reader knows how far to trust it.

A launcher is the human's own process, started in your own terminal. It is never a coding-agent session: `klatch worker
issue` refuses to run inside one, by design, and nothing here works around that. The order of a launcher's work is in
[FACTORY.md](../../FACTORY.md).

## 1. A plain script as launcher

[launcher-demo.sh](launcher-demo.sh) opens a run, issues three workers, hands work between them, contends for a path, ends the
run and revokes a worker. It runs against the real binary in CI, so every command in it is known to work.

```
KLATCH_BIN=/path/to/klatch clients/recipes/launcher-demo.sh
```

Copy it and replace the three `worker N post/claim` lines with your own worker commands. A worker is told one thing: the path
of its bundle, in `KLATCH_IDENTITY_FILE`. Never put a bundle in a repository.

## 2. A Makefile as launcher

The same commands as the script, one make target per step, so `make -j3 work-1 work-2 work-3` runs three workers at once and a
second `make done-1` frees the path for the next. Variables: `KLATCH` (the binary), `BUNDLES` (where bundles go).

```make
KLATCH ?= klatch
BUNDLES ?= bundles

issue:
	$(KLATCH) run create demo
	$(KLATCH) worker issue --count 3 --prefix w --clearance internal --parent factory --ttl 600 --out-dir $(BUNDLES)

work-%:
	KLATCH_IDENTITY_FILE=$(BUNDLES)/w-$*.bundle $(KLATCH) claim run.demo.main src/$*.rs --anchor wo:$*
	KLATCH_IDENTITY_FILE=$(BUNDLES)/w-$*.bundle $(KLATCH) post --kind work-order --payload '{"item":"wo:$*"}' run.demo.main "w-$* took src/$*.rs"

done-%:
	$(KLATCH) anchor done wo:$*

end:
	$(KLATCH) run end demo --archive demo.archive
```

Run `make issue`, then the work targets, then `make done-1 done-2 done-3`, then `make end`. Add a `revoke-%` target running
`$(KLATCH) worker revoke w-$*` if a worker may outlive its item.

What was checked: each `klatch` command above uses a subcommand and flags that `launcher-demo.sh` runs in CI (same words, not
the same Makefile), and `make -n` of every target prints its commands and runs nothing. What was not checked: the Makefile
itself against the real binary, so run `make issue` against a throwaway `KLATCH_HOME` first (see the demo's `env -i` lines
for the clean environment a worker should have).

## 3. The team launcher

`proto/scripts/team` runs each line of a tasks file as its own headless Claude Code session that is a verified member, with its
own worktree and branch.

```
proto/scripts/team smoke        # one tiny worker in a throwaway home and repo, never your real room
proto/scripts/team start TASKS_FILE
proto/scripts/team status
proto/scripts/team stop
```

Run it in your own terminal. Its settings (budget caps, the allowlist, relaunch limits) are in the header of the script; read
that header before `start`. What was checked here: the script and the four commands above exist and are in its header.
`proto/scripts/test-team.sh` is the script's own test.

## 4. The integrator

`proto/scripts/integrator` merges the pull requests that members queue with `ready: #N`. It needs the `integrator` role, a
dedicated clone and a `gh` login; [INTEGRATOR.md](../../proto/scripts/INTEGRATOR.md) lists what the human does. Check the setup
first, then start it:

```
INTEGRATOR_REPO=/path/to/dedicated/clone proto/scripts/integrator --dry-run
INTEGRATOR_REPO=/path/to/dedicated/clone proto/scripts/integrator --once
```

`--dry-run` posts, claims, pushes and merges nothing; `--once` drains the queue and exits. What was checked here: the script,
both flags and the guide exist; the script's own behaviour is tested by `proto/scripts/test-integrator.sh`.

## 5. The ask supervisor

`proto/scripts/ask-supervisor` starts a helper you configured for an ask nobody has answered, only when klatch says that helper
could answer. It issues no credential; the helper identities are yours, made once. See `INTEGRATION.md` section 9.

```
proto/scripts/ask-supervisor --dry-run
proto/scripts/ask-supervisor --once
```

`--dry-run` prints what it would start and starts nothing. What was checked here: the script and both flags exist; the script's
own test is `proto/scripts/test-ask-supervisor.sh`.

## What is still not here

A recipe for each third-party orchestrator that was actually run (#414), the table of overlaps verified by those runs, and the
choice of one general framework to try. Each needs a person to install and drive the tool. The recipe format for when they are
run is at the end of [ORCHESTRATORS.md](ORCHESTRATORS.md).
