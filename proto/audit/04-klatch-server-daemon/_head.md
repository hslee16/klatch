## `klatch-server daemon` (#102)

One gateway (`SharedGateway`, #140) over a local Unix socket; `SharedGateway::session_backend()` is the `session::Backend`
the session loop runs over. Every request a connection makes is a wire method of the table above, dispatched by the same
`handle` on the owner thread as the connection's principal, so it meets exactly the guards, the role check and the live
credential check of that method. The gateway is built with `for_members()`, as `serve` builds it, so no admin act is
reachable except the roles' own. What is new is who the connection is and what it may do before a frame is read.

| Evidence or path | What it gives | Check |
|---|---|---|
