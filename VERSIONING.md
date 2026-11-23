# Versioning

klatch has three version numbers, and they mean different things.

## 1. The release version (binary, crate, packages)

Wire protocol version and store schema version below are separate. The release version is the number on the binary, the
crates.io crate `klatch`, the PyPI package `klatch`, the npm package `@klatch/client` and the Homebrew formula.

- **SemVer.** While the major version is 0, a minor bump may break things; from 1.0 a breaking change needs a major bump.
- **Pre-releases.** Until the zero trust release gate is met (outside review, `DESIGN.md` section 23) every release is a
  marked pre-release. crates.io, npm, the archive names and `klatch --version` spell it in SemVer; PyPI and the git tag
  spell the same release in PEP 440. `pip install klatch` and `npm install @klatch/client` still resolve a pre-release
  while it is the only version; `cargo install klatch` needs `--version`.

  | Kind | SemVer (crate, npm, archive) | PEP 440 (PyPI, wheel) | Tag |
  |---|---|---|---|
  | alpha | `0.2.0-alpha.1` | `0.2.0a1` | `v0.2.0a1` |
  | beta | `0.2.0-beta.1` | `0.2.0b1` | `v0.2.0b1` |
  | release candidate | `0.2.0-rc.1` | `0.2.0rc1` | `v0.2.0rc1` |
  | dev | `0.1.0-dev.2` | `0.1.0.dev2` | `v0.1.0.dev2` |
  | stable | `0.2.0` | `0.2.0` | `v0.2.0` |

  The mapping is written once, in `release/pep440.sh`. `release/check-versions.sh`, the three checks in
  `build-release.yml` and the tests source it. `install/install.sh` is downloaded alone, so it carries a copy of the same
  block; `release/test-pep440.sh` fails if the copy differs. A label outside the table (`-gamma.1`, `-alpha`, build
  metadata) is refused, never passed through. The archive for a SemVer version `V` is `klatch-V-<target>.tar.gz`.
- **Ordering.** The first release is `0.2.0-alpha.1` (PyPI `0.2.0a1`), and `0.1.0-alpha.1` must not be used. Both
  registries sort by number first, so every `0.2.0-*` sorts above every `0.1.0-*`, including the published
  `0.1.0-dev.2` on crates.io and npm and `0.1.0.dev2` on PyPI. `0.1.0-alpha.1` would sort below `0.1.0-dev.2` under
  SemVer (labels compare as text, and "alpha" is before "dev"), so a user on the dev build would never be offered it.
  The two schemes also disagree about `dev` on the same `X.Y.Z` (SemVer puts `-dev.1` above `-alpha.1`, PEP 440 puts
  `.dev1` below `a1`), so one `X.Y.Z` line uses one kind, in the order alpha, beta, rc, final.
- **One release, one number.** A release tag publishes the binaries, PyPI, npm and crates.io together and each job fails
  if its version does not match the tag, so the three registries always carry the same number. The client libraries
  share the binary's release number only because of that; compatibility between a client and a binary is decided by
  the protocol version, not by the package version.
- `klatch --version` prints it with the other two numbers: `klatch 0.1.0-dev.2 (protocol 1, store schema 41)`.

## 2. The wire protocol version

Wire protocol version: 1

The newline-delimited JSON protocol of [PROTOCOL.md](PROTOCOL.md). Within a version it only grows: new optional fields
and new frame types are added, and a client must ignore a field or frame it does not know (typed handoffs added `kind`
and `payload` this way). A frame that is not `"v":1` is a protocol error. A change that an old client could not ignore
needs a new protocol version, which is a major version of klatch. How long a binary would keep speaking an older
protocol has not been decided.

## 3. The store schema version

Store schema version: 41

The version of the SQLite database layout. A store is migrated forward the first time a newer binary opens it, and
**an upgrade is one way**: an older binary refuses a store written by a newer schema with a message that says so, and
there is no downgrade. To keep the option, take a backup before upgrading (`klatch admin backup`, which needs a real
terminal) and restore it with `klatch admin restore` if you must go back. The key that encrypts the store does not
change with the schema.

To start a migration, run `proto/scripts/next-schema` first: it prints the next free number from origin/main and the
open pull requests, so two PRs do not claim the same one (#1070). `SCHEMA_VERSION` is the length of the migration list,
so the step, its list line and the line above are the only places the number appears.

## What this does not cover

Support windows (how long a release is backported), yanking and the rollback plan are tracked in the distribution epic
(#439) and are not written yet.
