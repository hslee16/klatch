## `klatch-server channel --webhook` intake (#1058)

`POST /github` on a loopback listener of the `channel` front end, for `gh webhook forward`. It is not a member and calls no gateway method: a verified delivery becomes a channel notice with a fixed template, never an operation on the store. Its one proof is the HMAC, not a worker credential.

| Evidence or path | What it gives | Check |
|---|---|---|
