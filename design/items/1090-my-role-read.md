A member can read its own role (#1090; no schema change; 2026-11-21).
No command showed whether a member holds a role, so the integrator dry run could only infer the `integrator` role from a recorded refusal of the `merge:main` claim. `Gateway::my_role` returns the caller's own role as its fixed word (`librarian`, `admin`, `reviewer`, `facilitator`, `integrator`) or none, and `klatch role` prints it (`none` without one).

It reads the caller's own `role:<member>` policy entry and takes no member parameter, so one member cannot read another's role. It is classified as OWN_RECORD, refused after the credential ends like `scope_of`, and writes no audit row. No wire method yet (ticket filed).
