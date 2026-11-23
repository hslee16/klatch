
## `GET /v1/ws` (#112, epic #13)

The klatch wire over one WebSocket connection, on the listeners that exist (`klatch-server http` on loopback, `klatch-server https`
behind mutual TLS): the upgrade is made inside `wire_http::serve_connection`, the session is the one `GET /v1/stream` runs, and the
frames are the same JSON lines, one per text message each way. No new listener, no new gateway method and no new `PeerEvidence`
variant. A WebSocket off loopback is always the TLS listener, so the refusals of the section above hold for it. `ws://` exists only on
the loopback listener.

| Evidence or path | What it gives | Check |
|---|---|---|
