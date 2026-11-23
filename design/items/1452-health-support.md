Code health K11: support modules, first slice (#1452, epic #1441; 2026-11-23).
A behaviour-preserving cleanup of three klatch files. No public API, schema, output or dependency change; the existing tests (admin shell word splitting, anchor file names, label key names) pass unchanged and no test was red first, because a pure refactor has none.

| File | cs before | cs after | Functions changed |
|---|---|---|---|
| `proto/klatch/src/admin/shell.rs` | 8.93 | 10.0 | `split_words` (quote and escape readers extracted: `read_single`, `read_double`, `read_escape`) |
| `proto/klatch/src/anchor.rs` | 9.24 | 9.68 | `file_name` (`is_file_safe`), `WorktreeAnchors::is_done` (`names_a_worktree`) |
| `proto/klatch/src/keys/labels.rs` | 9.38 | 9.68 | `label_key_name` (`is_key_name_char`) |

Left alone on purpose: everything in proof/ (the passphrase proof, five security reviews), keys custody beyond the one predicate, admin acts and prompts, member identity, harness and init (consent code), where a refactor is not clearly behaviour-preserving without golden tests first; `router.rs` and `jev.rs` (ask-routing work #1407 in flight); test files (assertion blocks); `client/http.rs`. The rest of the cluster stays open on #1452.
