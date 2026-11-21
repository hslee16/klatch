## CLI commands

Commands that act as a member go through `run_as_member`: the caller is identified first (a key bundle, else a
session found from the process tree or a typed name, never trusted for more than `Unattributed`), the gateway is
built with `ForbidFloors(AllowAll)` and `for_members()`, and every command calls the gateway through the `Direct`
transport with the caller's principal. So each of these meets exactly the checks of the wire method or gateway call it
wraps.

| Command | Gateway call | Check |
|---|---|---|
