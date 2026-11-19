# Versioning

klatch has three version numbers, and they mean different things.

## 1. The release version (binary, crate, packages)

Wire protocol version and store schema version below are separate. The release version is the number on the binary, the
crates.io crate `klatch`, the PyPI package `klatch`, the npm package `@klatch/client` and the Homebrew formula.

- **SemVer.** While the major version is 0, a minor bump may break things; from 1.0 a breaking change needs a major bump.
- **Pre-releases.** Until the zero trust release gate is met (outside review, `DESIGN.md` section 23) every release is a
  marked pre-release: `0.1.0-dev.N` for crates.io and npm, `0.1.0.devN` for PyPI (PEP 440 spells the same release
  differently), tag `v0.1.0.devN`. `pip install klatch` and `npm install @klatch/client` still resolve a pre-release
  while it is the only version; `cargo install klatch` needs `--version`.
- **One release, one number.** A release tag publishes the binaries, PyPI, npm and crates.io together and each job fails
  if its version does not match the tag, so the three registries always carry the same number. The client libraries
  share the binary's release number only because of that; compatibility between a client and a binary is decided by
  the protocol version, not by the package version.
- `klatch --version` prints it with the other two numbers: `klatch 0.1.0-dev.2 (protocol 1, store schema 32)`.

## 2. The wire protocol version

Wire protocol version: 1

The newline-delimited JSON protocol of [PROTOCOL.md](PROTOCOL.md). Within a version it only grows: new optional fields
and new frame types are added, and a client must ignore a field or frame it does not know (typed handoffs added `kind`
and `payload` this way). A frame that is not `"v":1` is a protocol error. A change that an old client could not ignore
needs a new protocol version, which is a major version of klatch. How long a binary would keep speaking an older
protocol has not been decided.

## 3. The store schema version

Store schema version: 32

The version of the SQLite database layout. A store is migrated forward the first time a newer binary opens it, and
**an upgrade is one way**: an older binary refuses a store written by a newer schema with a message that says so, and
there is no downgrade. To keep the option, take a backup before upgrading (`klatch admin backup`, which needs a real
terminal) and restore it with `klatch admin restore` if you must go back. The key that encrypts the store does not
change with the schema.

## What this does not cover

Support windows (how long a release is backported), yanking and the rollback plan are tracked in the distribution epic
(#439) and are not written yet.
