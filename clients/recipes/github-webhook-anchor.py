#!/usr/bin/env python3
"""Check a GitHub webhook's signature, then pipe the body to `klatch anchor github` (#531).

    github-webhook-anchor.py --secret-file PATH --event "$X_GITHUB_EVENT" --signature "$X_HUB_SIGNATURE_256" < body

`klatch anchor github` verifies nothing: it trusts what it is given. This is the step in front of it for a payload that
arrives over a network. Whatever receives the webhook (a reverse proxy, a small web handler, a CI job) hands this script
the raw body on standard input and the values of two headers; the script does three things and nothing else.

1. It checks `X-Hub-Signature-256`: `sha256=` and the lowercase hex HMAC-SHA256 of the raw body under the webhook's
   secret, compared in constant time. Anything else, including the old SHA-1 header's form and a missing header, is
   refused (exit 3) and nothing is forwarded.
2. It looks at `X-GitHub-Event`. `pull_request` and `issues` are the events `klatch anchor github` reads; a signed
   `ping` or any other event is acknowledged and not forwarded (exit 0), so klatch is never given a payload it would
   refuse whole.
3. It runs `klatch anchor github` with the body on its standard input, unchanged, and exits with its status.

The secret is read from a file, never from the command line (a command line is visible to every process of the machine).
The body is read with a size limit before anything is computed. Neither the secret, the body nor the expected signature is
ever printed. Exit codes: 0 done or ignored, 2 a usage or configuration error, 3 a bad signature, 4 a body over the limit.
"""
import argparse
import hashlib
import hmac
import re
import subprocess
import sys

FORWARDED_EVENTS = ("pull_request", "issues")
DEFAULT_MAX_BYTES = 5 * 1024 * 1024
_SIGNATURE = re.compile(r"sha256=[0-9a-f]{64}\Z")


def verify(secret, body, header):
    """True only when `header` is `sha256=` plus the lowercase hex HMAC-SHA256 of `body` under a non-empty `secret`."""
    if not secret or not header or not _SIGNATURE.match(header):
        return False
    expected = "sha256=" + hmac.new(secret, body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected.encode(), header.encode())


def read_secret(path):
    """The secret in the file with trailing line breaks removed, or None when it cannot be read or is empty."""
    try:
        with open(path, "rb") as f:
            secret = f.read().rstrip(b"\r\n")
    except OSError:
        return None
    return secret or None


def read_body(limit):
    """The whole standard input, or None when it is longer than `limit` bytes."""
    body = sys.stdin.buffer.read(limit + 1)
    return None if len(body) > limit else body


def parser():
    p = argparse.ArgumentParser(description="Verify a GitHub webhook signature, then run `klatch anchor github`.")
    p.add_argument("--secret-file", required=True, help="a file holding the webhook secret (never the secret itself)")
    p.add_argument("--signature", help="the value of the X-Hub-Signature-256 header")
    p.add_argument("--event", help="the value of the X-GitHub-Event header")
    p.add_argument("--klatch", default="klatch", help="the klatch program to run (default: klatch on the PATH)")
    p.add_argument("--max-bytes", type=int, default=DEFAULT_MAX_BYTES, help="refuse a larger body (default 5 MiB)")
    return p


def main(argv):
    args = parser().parse_args(argv)
    secret = read_secret(args.secret_file)
    if secret is None:
        print("github-webhook-anchor: the secret file is missing, unreadable or empty", file=sys.stderr)
        return 2
    if not args.event:
        print("github-webhook-anchor: --event (the X-GitHub-Event header) is required", file=sys.stderr)
        return 2
    body = read_body(args.max_bytes)
    if body is None:
        print(f"github-webhook-anchor: the body is over {args.max_bytes} bytes", file=sys.stderr)
        return 4
    if not verify(secret, body, args.signature):
        print("github-webhook-anchor: the signature does not match; nothing was forwarded", file=sys.stderr)
        return 3
    if args.event not in FORWARDED_EVENTS:
        print(f"github-webhook-anchor: ignored a signed {args.event!r} event", file=sys.stderr)
        return 0
    try:
        return subprocess.run([args.klatch, "anchor", "github"], input=body).returncode
    except OSError as e:
        print(f"github-webhook-anchor: could not run {args.klatch}: {e.strerror}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
