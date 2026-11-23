CodeScene components are one checked-in partition (#1457; no schema change; 2026-11-23). `.codescene/components.json` defines 15 architectural components that partition every tracked source file (`*.rs`, `*.py`, `*.ts`, `*.sh`, shebang scripts) exactly, with application code and its unit tests in the same component and the integration tests directory its own; no file may be in two components because CodeScene's behaviour for an overlap is not documented. `proto/scripts/codescene-components` prints the CSV for the UI import and the REST payload, `check` verifies the exact partition against `git ls-files` (it runs in the local gate through `test-codescene-components.sh`, so new directories must be assigned), and `apply` GETs the existing components, shows the diff and PUTs only with `--yes` (the PUT replaces everything); the token comes only from `CODESCENE_TOKEN` or a mode-0600 file, never an argument, and is never printed. Design and docs hold no source files and are not components.

    Components and globs:

    - `gateway` (154 source files): `proto/klatch/src/gateway/**`
    - `store` (92 source files): `proto/klatch/src/store/**`
    - `domain` (65 source files): `proto/klatch/src/domain/**`
    - `session-harness` (48 source files): `proto/klatch/src/session/**`, `proto/klatch/src/harness/**`
    - `commands-args` (16 source files): `proto/klatch/src/commands/**`, `proto/klatch/src/args/**`
    - `security` (40 source files): `proto/klatch/src/proof/**`, `proto/klatch/src/keys/**`, `proto/klatch/src/admin/**`, `proto/klatch/src/member/**`
    - `jev` (23 source files): `proto/klatch/src/jev.rs`, `proto/klatch/src/jev/**`
    - `klatch-core` (37 source files): `proto/klatch/src/anchor.rs`, `proto/klatch/src/client.rs`, `proto/klatch/src/codex.rs`, `proto/klatch/src/consent.rs`, `proto/klatch/src/init.rs`, `proto/klatch/src/init_codex.rs`, `proto/klatch/src/judge.rs`, `proto/klatch/src/lib.rs`, `proto/klatch/src/main.rs`, `proto/klatch/src/mediator.rs`, `proto/klatch/src/roles.rs`, `proto/klatch/src/router.rs`, `proto/klatch/src/run.rs`, `proto/klatch/src/shuttle_tests.rs`, `proto/klatch/src/sync.rs`, `proto/klatch/src/worker.rs`, `proto/klatch/src/client/**`, `proto/klatch/src/codex/**`, `proto/klatch/src/doctor/**`, `proto/klatch/src/kani_proofs/**`, `proto/klatch/src/protocol/**`, `proto/klatch/src/roles/**`, `proto/klatch/src/shuttle_tests/**`, `proto/klatch/fuzz/**`
    - `klatch-integration-tests` (110 source files): `proto/klatch/tests/**`
    - `klatch-server` (20 source files): `proto/klatch-server/**`
    - `klatch-bench` (72 source files): `proto/klatch-bench/**`
    - `clients` (23 source files): `clients/**`
    - `coordbench` (158 source files): `experiments/coordbench/**`
    - `experiments-other` (28 source files): `experiments/crypto-spike/**`, `experiments/dm-adoption/**`, `experiments/gates/**`, `experiments/headless/**`, `experiments/hegel-bench/**`, `experiments/identity/**`, `experiments/turso-spike/**`, `experiments/variance/**`
    - `release-and-scripts` (68 source files): `release/**`, `proto/scripts/**`, `install/**`, `proto/linux-keys/**`, `proto/linux-discovery/**`, `proto/check-notices.sh`
