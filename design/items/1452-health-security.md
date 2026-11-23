Code health K11, security slice (#1452, epic #1441; 2026-11-23).
A behaviour-preserving cleanup of five security-adjacent files, done after the golden tests of #1480 landed. No public API, schema, SQL, stored byte, wire, CLI output, error text, audit row or prompt text change. The golden tests and the unit tests pass unchanged and were never regenerated. No test was red first, because a pure refactor has none; each function touched is pinned by a golden or an existing test (see the table).

| File | cs before | cs after | Functions changed | Pinned by |
|---|---|---|---|---|
| `proto/klatch/src/admin/acts.rs` | 9.39 | 10.0 | `policy` (arms extracted: `room_creation`, `rate_limit`, `member_rate_limit`) | audit rows golden, authz audit; the printed lines are moved verbatim |
| `proto/klatch/src/admin/prove.rs` | 9.53 | 10.0 | `describe_words` (`cut_word`, `cap_words`) | `golden_admin_prompts` (argument sanitiser) |
| `proto/klatch/src/admin/mod.rs` | 9.38 | 10.0 | `run` (`run_store_less`, `run_proved`; same order: describe, db path, route, output check, proof, confirm, act) | `golden_admin_*`, `admin_proof*` |
| `proto/klatch/src/init.rs` | 9.15 | 10.0 | `set_session_end`, `sync_stop`, `set_optional` (`either`, `any_on`, `post_tool_use_flags`, `sync_optional`) | `golden_consent_hooks`, `init*`, `consent` |
| `proto/klatch/src/member/identity.rs` | 9.38 | 10.0 | `identify`, `identify_by_ssh_key` (`declared_name`, `discover_asked`, `names_itself`) | `authz_audit`, identity survivor tests |

Left alone on purpose:

- `admin/route.rs` (9.35, `Route::from`, 118 lines): a flat exhaustive match over every admin command that decides which act family (and so which proof path) a command takes. Splitting it by family needs a catch-all arm, which turns a missing route from a compile error into a runtime panic. Not worth it for a security routing table.
- `proof/mod.rs` `prove` (9.27), `proof/tty.rs` `read_tty_line` (9.68), `proof/verifier.rs` `Verifier::parse` (9.24): handle the passphrase bytes, the terminal restore, or the verifier format. Out of scope by the ticket's rules. `proof/window.rs` and `proof/filelimit.rs` are already 10.0.
- `keys/bundle.rs` `Bundle::read` (9.68), `keys/linux.rs` (9.09, test assertion blocks plus a keyring backend), `keys/labels.rs`, `member/custody/label_rotation.rs` (`repair_label_key`), `member/custody/label_rotation_tests.rs`: rotate-label-key, the bundle bytes and the Linux keyring backend are not pinned by the goldens; no characterisation test was added, so the functions are untouched.
- `harness/conformance.rs` (`check_harness`) and `harness/probe.rs` (`probe`, 6 arguments): the conformance text is not pinned.
- `member/scope.rs` `ensure_member_attrs` (5 arguments): the only route is a signature change of a crate-internal function used from `member/mod.rs`; left.
- `member/scope/attr_tests.rs`: assertion blocks in tests.

`init.rs` is over the 600-line cap (1494 lines before this change, a few more after); splitting the file is a separate change.
