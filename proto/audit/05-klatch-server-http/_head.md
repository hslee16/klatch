## `klatch-server http` (#1059)

The klatch wire over HTTP and server-sent events, on loopback only. A stream (`GET /v1/stream`) runs the session loop of
`klatch serve --stdio` over the same `SharedGateway` as the daemon; `POST /v1/frames` writes frames into a session. Every
frame is a wire method of the table above, run as the session's principal on the owner thread, so it meets the same guards,
role checks and live credential check. No new gateway method and no new `PeerEvidence` variant: the evidence is the existing
`PeerEvidence::Credential` (`Bearer` and the evidence table of item 53 stay for a later transport).

| Evidence or path | What it gives | Check |
|---|---|---|
