## `klatch admin`

The human's controls. `authorize` in `admin/mod.rs` refuses unless standard input and output are a terminal and no
coding-agent session is above the process, then asks for the target typed back for the destructive ones. That is a
heuristic, not proof (#318). Past it, the commands in the first family reach the gateway as a principal and meet
`administer` (the guard, the role check and an audit row); the second family acts on the key or the store file
directly and meets only the terminal heuristic.

| Family | Commands | Check after the terminal heuristic |
|---|---|---|
