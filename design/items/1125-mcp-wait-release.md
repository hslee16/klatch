A klatch_wait over POST /mcp ends when the server stops or the client hangs up (#1125; no schema change; 2026-11-21).
**Question.** A klatch_wait over `POST /mcp` runs inside its connection handler for up to 55 s. How do SIGTERM and a departed client stop holding it?

**Decision.** `McpServer::with_cancel` takes a check that the wait loop asks between its 100 ms polls; true ends the wait as a timeout with no messages (none were taken, so none are lost, and the client may call again). The `/mcp` handler passes a check of the daemon stop flag and of the client socket (a non-blocking `recv` with `MSG_PEEK`, which reads end-of-file once the client has closed; nothing is consumed and the socket mode is untouched). So shutdown and the connection slot are released within about 100 ms of the event, not at the end of the wait.

**Tests.** `tests/mcp_http.rs`: a client that leaves mid-wait frees its slot (cap 1) within 5 s; SIGTERM mid-wait stops the process within 5 s. Both were red before the change.
