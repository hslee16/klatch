## `klatch admin`

The human's controls. Every command first asks for the admin passphrase on a real terminal (#1233; `prove::require`,
`klatch::proof`): an Argon2id verifier kept in policy, a failure limit that is charged before the attempt is checked,
audited by count only, and an unlock window held in the memory of one process (`klatch admin shell`). The proof
makes the principal `human` (`Attribution::Verified`, a reserved name) and the gateway's guard `RequireHuman` accepts
nothing else; the destructive commands then ask for the target typed back. The terminal and process-tree heuristic
(`gate`) is gone. Past the proof, the commands in the first family reach the gateway as that principal and meet
`administer` (the guard, the role check and an audit row); the second family acts on the key or the store file and
meets the same guard through `admin_authorize` before it touches either, and takes the proof as an argument.
What it does not stop (a same-user process that reads the store and its key directly, one that watches the keyboard
or the memory of an unlocked process) is in SECURITY.md and ticketed.

| Family | Commands | Check after the proof |
|---|---|---|
