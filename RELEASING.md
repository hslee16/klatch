# Releasing

How a release is made. The workflow is `.github/workflows/release.yml`; the rules are in [VERSIONING.md](VERSIONING.md).
Publishing needs the maintainer: the `release` environment approval in GitHub Actions, and the approval of the staged
npm version on npmjs.com with a security key. Nothing here can be done by an agent alone, by design.

## 1. Pick the number and bump every place it lives

One release is one number, spelled three ways (`0.1.0-dev.2` for the crate and npm, `0.1.0.dev2` for PyPI, tag
`v0.1.0.dev2`). Change all of these in one pull request:

- `proto/Cargo.toml` (the workspace version) and `proto/Cargo.lock` (`cargo build` updates it)
- `clients/python/pyproject.toml` and `clients/python/package/wheel_init.py`
- `clients/typescript/package.json` and `clients/typescript/package-lock.json`
- `CHANGELOG.md`: move the Unreleased entries under the new version and date
- `README.md`: the version in the `cargo install` line and the pre-release notice
- `VERSIONING.md`, if the protocol or store schema version changed

Run `release/check-versions.sh`; CI runs it on every pull request, and each registry job repeats the check against the
tag, so a mismatch stops the release before anything is published.

## 2. Merge, tag, approve

1. CI green, merge to `main`.
2. Create a GitHub **pre-release** (while every version is a pre-release) with the tag `v<PyPI spelling>`, from `main`.
3. In Actions, open the run and approve the `release` environment once: it covers PyPI, npm staging, crates.io and the
   binaries. A crates.io version can never be retried, so read the build jobs first.
4. On npmjs.com, Staged Packages, approve the new `@klatch/client` version with your key.

If a job fails, rerun one registry with the manual trigger (`Run workflow`, a tag and a target of `npm`, `crates` or
`binaries`); a run started by a release uses the workflow file as it was at the tag. A published PyPI or crates.io
version is not republished: bump to the next number.

## 3. Verify

- PyPI shows the version; `pip install klatch==<version>` imports.
- npm shows it as `latest` with provenance; crates.io shows it with the trusted publishing data.
- The release has the six archives, six SBOMs, `install.sh`, `SHA256SUMS` and `SHA256SUMS.sigstore.json`. Check one
  with the commands in [SECURITY.md](SECURITY.md) ("Verifying a download").

## 4. Homebrew

The formula in `hslee16/homebrew-klatch` is bumped by hand: the version, the four download URLs and the four SHA-256
values from the release's `SHA256SUMS` (`Formula/klatch.rb`). Open a pull request there; its CI audits, installs and
tests the formula on macOS and Linux.

## Not written yet

Yanking and the rollback plan, and the dry run of the whole release on a pull request (#437).
