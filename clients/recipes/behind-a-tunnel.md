# Reaching a loopback-only klatch-server through a tunnel or a TLS terminator

> **Native mutual TLS now exists (#108, experimental).** `klatch-server https --listen 127.0.0.1:7443 --tls-cert PEM --tls-key PEM
> --tls-client-ca PEM` serves the same wire behind a mutual TLS listener, so a same-host terminator (pattern C below) is no
> longer the only way to give remote callers an encrypted, certificate-gated port. You supply the server certificate, its
> key (the file must be mode 0600) and a CA file for client certificates; klatch makes no certificates and runs no CA. A
> client without a certificate from that CA is refused at the handshake, and a certificate is still only a key to the port:
> the caller is the worker credential, exactly as in section 1. The listener is loopback unless `--allow-off-loopback` is
> given. **No CRL and no OCSP is checked** (#1491): revocation means short-lived certificates, or a new CA file and a
> restart. **Denial of service remains possible** from any peer that can reach the port (it can open connections without
> a certificate; the handshake deadline, the cap on connections in the handshake and the stall deadline bound it, they do
> not remove it: when the cap is full of young handshakes a newcomer is the one refused, and a flood that keeps it full keeps
> newcomers out. The cost to the attacker is low: no certificate, and only a few connections per second, roughly 5 to 6 per
> second at the defaults, was enough in a review to starve every honest client; it was equally cheap before the age rule,
> which only protects a client already mid-handshake from eviction. The remedies are a firewall, a tunnel or loopback, and
> the per-source limit of #1493), so keep `--allow-off-loopback` off unless you need it and put a firewall or a tunnel in front. Details
> and limits: `design/items/108-tls-listener.md`. The tunnel and terminator patterns below remain valid and need none of this.

`klatch-server http` (the wire protocol over HTTP and server-sent events, and MCP at `POST /mcp`) listens on loopback only
and refuses any other address, naming the zero-trust prerequisites #157 and #161. This page is the safe way to reach it
from another machine today **without changing that**: klatch keeps listening on `127.0.0.1` or `[::1]`, and something else
carries the remote caller to that loopback address over an encrypted, authenticated channel. It is the deployment guide of
#110. klatch never trusts a proxy's word about identity (the owner's decision of 2026-11-23), so there is no forwarded-identity half to build; see section 1.

**What is tested and what is not.** The klatch half is tested: `proto/klatch-server/tests/http_wire.rs` shows that
forwarded-identity headers give no identity and change none (every route is still `401` without a credential, and a
credential for one member with headers naming another is the first member), `proto/klatch-server/tests/route_audit.rs`
shows no source file reads such a header or a peer address, `tests/forwarded_audit.rs` shows the count by header name and that no header value reaches the log, and the loopback refusal is tested in `http_wire.rs` and
`loopback_tests.rs`. The ssh and stunnel lines below were **not run** by the people who wrote this guide: ssh not run,
stunnel not run. Treat every ssh, sshd and stunnel option as documentation to verify against your own version's manual
before you rely on it.

## 1. The rule that makes every pattern safe

- **Who the caller is.** The caller is the worker credential: `Authorization: Bearer TOKEN` with `X-Klatch-Member: MEMBER`,
  a credential the human issued, checked live on every request (it expires, renews on use up to a cap, and a revoke stops
  it at its next request). The tunnel or terminator authenticates a *machine or person to the tunnel*; it never names a
  klatch member. A client certificate that a terminator verified is a gate in front of the port, not an identity inside
  klatch.
- **Forwarded headers are ignored, and counted.** Identity always comes from the worker credential, never from a proxy.
  `klatch-server` never reads the value of `Forwarded`, `X-Forwarded-For`, `X-Forwarded-User`, `X-Forwarded-Email`,
  `X-Forwarded-Client-Cert`, `X-Real-IP`, `X-Remote-User`, `X-Client-Cert`, `X-Client-Verify`, `X-Amzn-OIDC-Identity`, or any
  `X-SSL-Client-*` header. A proxy cannot assert a member, and a request from the proxy's loopback address gets no more
  trust than one from anywhere else: with no credential it is `401`. Such headers are not an error. A request that carries
  one or more of them is counted once, and each name it carried is counted once, so an operator can see a proxy adding
  them or a client forging them. The count holds names and numbers only, never an address, a user or a certificate text.
  It is counted in memory. While the server runs it shows as one line on stderr, at most once a minute and only when the
  count changed (`klatch-server: forwarded identity headers seen so far: requests=3 x-forwarded-for=3`); when the server
  stops, as one summary line (`klatch-server: forwarded identity headers ignored: requests=3 x-forwarded-for=3`). It is
  also kept: one `forwarded-ignored` audit row every ten minutes (only when the count changed) and one at shutdown, each
  holding what was counted since the previous row, so the rows add up across restarts (`klatch admin audit --action forwarded-ignored` shows them; the default `klatch admin audit` leaves them out and says how many it hid).
  A request never writes a row; a timer does (#1490).
- **The token is a secret on the wire.** The Bearer token is in every request, so it must only ever cross a link that is
  encrypted end to end between the two machines. That is what the tunnel is for.
- **Plaintext only on the same host.** A hop that is plaintext (the terminator talking to klatch) is acceptable only on the
  same host, over loopback or a Unix socket. There is never a plaintext hop between hosts: if the terminator and klatch are
  on different machines, re-encrypt the second hop, and klatch itself will not listen off loopback to help with that
  (never `0.0.0.0`; the bind is refused).

## 2. Pattern A: ssh forced command (no port at all)

If the remote side can run a command, prefer [ssh-transport.md](ssh-transport.md): `ssh host klatch serve --stdio` with the
member fixed by a forced command in `authorized_keys`. klatch opens no socket and the member comes from the key, not from a
header. Use this pattern when you can.

## 3. Pattern B: an ssh local forward to the loopback listener

On the server, as the klatch account, start the listener on loopback (the address has no default):

```
klatch-server http --listen 127.0.0.1:7600
```

On the client machine, forward a local loopback port to it over ssh (ssh not run; check the manual of your ssh):

```
ssh -N -L 127.0.0.1:7600:127.0.0.1:7600 klatch@klatch.example.org
```

The client then talks to `http://127.0.0.1:7600` with its Bearer token and `X-Klatch-Member`. The `http://` is plain only
on the client's own machine; the ssh channel is what crosses the network. Verify the host key (never
`StrictHostKeyChecking=no`; see the host key section of ssh-transport.md). On the server, an `authorized_keys` line for a
tunnel-only key can say `restrict,port-forwarding,permitopen="127.0.0.1:7600"` so the key can reach that port and nothing
else (verify on your sshd). Anyone who can reach the forwarded port on the client machine can *try* it, and still needs a
credential; do not forward it to `0.0.0.0` (do not add a `-L` bind address other than loopback).

## 4. Pattern C: stunnel with mutual TLS, on the same host as klatch

A TLS terminator on the server host accepts connections from other machines with a client certificate required, and
connects onward to `127.0.0.1:7600` on the same host. The shape to build (stunnel not run; every name below is from memory
of stunnel's manual, check it against your version):

```
[klatch]
accept  = 0.0.0.0:7643
connect = 127.0.0.1:7600
cert    = /etc/stunnel/server.pem
CAfile  = /etc/stunnel/clients-ca.pem
verifyChain = yes
```

Invariants to check, whatever the spelling: the terminator requires and verifies a client certificate against your own CA
(a verification level that skips the check is the failure to avoid); it presents a server certificate the clients verify
against a name or a pin, not "accept anything"; it connects onward only to loopback on this host; and the port it accepts
on is the only one exposed. The terminator listening off loopback is the terminator's act, not klatch's; klatch is still
on loopback and refuses anything else (with `klatch-server https` klatch terminates TLS itself and no terminator is needed). A client uses a matching stunnel (or any mutual-TLS client) to reach it and then
sends its Bearer token as above.

**What klatch cannot do here.** Behind a terminator it cannot see the client certificate or the peer's address, so it cannot
tie a certificate to a member; with the native listener it can see the certificate and still does not tie it to a member
(the decision of #157). Either way the certificate decides who may reach the port and the credential decides who they are.
Two people sharing one client certificate are still two credentials. Certificate enrollment, rotation and revocation are
your CA's job; klatch has no part in them.

## 5. What is not built

- **Forwarded identity from configured proxy addresses**: not built and not going to be. The owner decided on 2026-11-23
  that klatch never trusts a proxy's word. The headers are ignored and counted (section 1), shown live and kept in the audit (#1490).
- **A certificate as an identity inside klatch**: not built, by decision (#157). Native mutual TLS exists (#108, experimental) and only gates the port. Certificate revocation lists, OCSP, and replacing the server certificate without a restart are not built.
- **A run across two machines**: needs a human and real hosts.
