
No new gateway method: `Gateway::verify_credential` already exists (GUARDED list unchanged, zero-trust lists unchanged), and
`session::Backend` is a trait over calls the session loop already made. The daemon opens the store through
`klatch::member::spawn_shared_gateway` (same environment variables, same custody as `klatch mcp --stdio`). Label keys and
the harness are per connection (#1096), neither a new method nor a new guard: for each request the owner reads from the
keystore the keys of the sealed labels that request's principal is cleared to (`sealed_labels_within`, the rule
`klatch serve --stdio` uses; no other key, none kept after the request, a failure to load is no key), and a connection that
never sent `identify` whose kernel-reported pid is inside a verified Claude Code session (`peer_runs_claude_code`, the walk a
command makes) joins as Claude Code, every other connection as the generic floor (typed name, #881; credential, #880).
Tests: `shared_session_tests` (keys asked only within clearance, none without a source, harness per connection) and
`tests/daemon.rs` (a sealed label over the socket, with and without a key in the keystore).

