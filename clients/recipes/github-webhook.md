# Recipe: a GitHub webhook releases the claims anchored to a pull request or issue

A claim can name the work item it is for (`klatch claim ROOM PATH --anchor gh:pr:OWNER/REPO/N`, or `gh:issue:OWNER/REPO/N`).
When the pull request is merged or closed, or the issue closed, `klatch anchor github` releases those claims at once instead
of at the end of the lease. It reads the payload from standard input and **verifies nothing**: it trusts what it is given,
and it never calls GitHub.

When the payload comes over a network, something has to check that GitHub sent it. `github-webhook-anchor.py` is that step.
It is a filter, not a server: whatever receives the webhook (a reverse proxy, a small web handler, a CI job) hands it the raw
body and two header values, and it either pipes the body to `klatch anchor github` or refuses.

```sh
github-webhook-anchor.py \
  --secret-file /path/to/webhook-secret \
  --event "$X_GITHUB_EVENT" \
  --signature "$X_HUB_SIGNATURE_256" \
  < body.json
```

## What it checks

1. **The signature.** The `X-Hub-Signature-256` header must be `sha256=` and the lowercase hex HMAC-SHA256 of the raw body
   under the webhook's secret (GitHub documents this scheme for webhook deliveries), compared in constant time. A missing
   header, a different algorithm (the older SHA-1 header's form), upper-case hex, a wrong length or a body changed by one
   byte is refused with exit 3 and nothing is forwarded. The check runs before the event is looked at and before klatch is
   started. An empty secret never verifies.
2. **The event.** Only `pull_request` and `issues` (what `klatch anchor github` reads) are forwarded. A signed `ping` (the
   delivery GitHub sends when a webhook is created) or any other event is acknowledged on standard error and not forwarded,
   with exit 0, so klatch is never handed a payload it would refuse whole. Subscribe the webhook to just those two events.
3. **The size.** The body is read with a limit (5 MiB by default, `--max-bytes`) before anything is computed; a larger body
   is refused with exit 4.

Exit codes: `0` done or ignored, `2` a usage or configuration error (no secret file, an empty secret, no `--event`, klatch
not found), `3` a bad signature, `4` a body over the limit; any other code is `klatch anchor github`'s own.

## Set up

1. Make a long random secret and keep it in a file only the launcher's user can read (`chmod 600`). The secret is never an
   argument (a command line is visible to every process of the machine) and is never printed, logged or committed; this
   repository holds none, and the tests use a made-up value.
2. Create the webhook on the repository: the payload URL of your receiver, content type `application/json` (so that the body
   that is signed is the JSON itself), the same secret, and only the **Pull requests** and **Issues** events.
3. Have the receiver pass the **raw** body (not a parsed and re-serialised one: the signature covers the exact bytes) and the
   `X-GitHub-Event` and `X-Hub-Signature-256` header values to the script, and answer the delivery with a success status
   when the script exits 0.
4. Run it as the launcher, the way `klatch anchor` is meant to run: not from inside a coding agent's session and not with a
   worker's key bundle in the environment (`klatch anchor` refuses both). It is the launcher's act, audited like any other.

A receiver written in Python can call it like this (no network code is in the recipe itself):

```python
import subprocess

def handle(headers, raw_body):
    done = subprocess.run(
        ["github-webhook-anchor.py", "--secret-file", SECRET_FILE,
         "--event", headers.get("X-GitHub-Event", ""),
         "--signature", headers.get("X-Hub-Signature-256", "")],
        input=raw_body, capture_output=True,
    )
    return 200 if done.returncode == 0 else 401 if done.returncode == 3 else 500
```

## What it does not do

- **It does not stop a replay.** A correctly signed delivery that is sent again verifies again. Here a replay is nearly
  harmless (releasing claims that are already released changes nothing), but a replayed old `closed` event would also release
  a newer claim anchored to the same item after it was reopened. The `X-GitHub-Delivery` header is unique per delivery; a
  receiver that wants to refuse repeats can remember the ones it has seen. This script does not.
- **It does not choose which repositories you trust.** Anyone holding the secret can sign a payload for any repository. Use one
  secret per webhook, and claim only with anchors for repositories you expect.
- **It is not a server.** TLS, rate limits and the listener are the receiver's.

## Tested

`clients/recipes/test_github_webhook_anchor.py` runs offline (`python3 -m unittest discover -s clients/recipes -p 'test_*.py'`;
the gate and CI run it): the example from GitHub's webhook documentation as a fixed vector, every malformed header above,
a changed body, a wrong secret, a missing or empty secret, an oversize body, the `ping` and unknown-event cases, that nothing
private is printed, and that klatch's own failure is passed on, all with a stand-in klatch. When a klatch binary is built it
also runs a real merge payload end to end: a claim anchored to the pull request is held, a signed payload arrives, the claim
is released. No test contacts the network.
