## `klatch-server https` (#108, epic #13, gate #157)

The routes of `klatch-server http` (`GET /v1/stream`, `POST /v1/frames`, `POST /mcp`) behind a mutual TLS listener, served by
the same handlers (`wire_http::serve_connection`), so every route is a row of the sections above and no route is new. No new
gateway method and no new `PeerEvidence` variant: a client certificate gates the port and is never mapped to a member. The
decisions of #157 (2026-11-23): rustls, operator-supplied certificates and a CA file, no enrollment, no CA in klatch.

| Evidence or path | What it gives | Check |
|---|---|---|
