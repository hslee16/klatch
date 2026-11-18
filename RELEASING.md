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
- `CHANGELOG.md`: put the entries under the new version and date: the Unreleased entries, plus what
  `release/changelog-fragments.sh collect` prints (then delete the files in `changelog.d/`)
- `DESIGN.md`: run `release/design-fragments.sh assemble`; it appends the fragments in `design/items/` as the next numbered
  items in ticket order and deletes them. It prints the number each ticket got; change references written as "the design
  item for #NNN" to the number
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

- PyPI shows the version with an sdist, a pure-Python wheel and four platform wheels; `pip install klatch==<version>`
  imports, and `pipx run klatch==<version> --version` (or `uvx`) prints the version line.
- npm shows it as `latest` with provenance; crates.io shows it with the trusted publishing data.
- The release has the six archives, six SBOMs, `install.sh`, `SHA256SUMS` and `SHA256SUMS.sigstore.json`. Check one
  with the commands in [SECURITY.md](SECURITY.md) ("Verifying a download").

## 4. Homebrew

The formula in `hslee16/homebrew-klatch` is bumped by hand: the version, the four download URLs and the four SHA-256
values from the release's `SHA256SUMS` (`Formula/klatch.rb`). Open a pull request there; its CI audits, installs and
tests the formula on macOS and Linux.

## Rolling back

Nothing published is deleted. A bad release is **replaced by the next number** (see VERSIONING.md) and the bad one is
marked so people stop picking it. In this order, so that nobody new is steered to it while you work:

1. **GitHub release.** `gh release edit <tag> --draft` hides it (the installer finds the newest release through the
   public list, so a drafted release is no longer found). To remove one bad file: `gh release delete-asset <tag> <name>`.
   Do not delete the tag while a registry version points at it. Build provenance attestations stay; they are true
   statements about the files they name.
2. **crates.io.** `cargo yank --version <version> klatch` stops new dependency resolutions; projects that already
   locked it keep working. `cargo yank --undo --version <version> klatch` reverses it. It needs an API token with the
   `yank-version` scope (create one at crates.io/settings/tokens, use it, revoke it): trusted publishing only publishes.
3. **PyPI.** On the project's release page choose **Yank release** and give a reason: pip skips a yanked version unless
   it is pinned exactly. Deleting a release is possible but cannot be undone, and the file names are never reusable.
4. **npm.** `npm deprecate @klatch/client@<version> "<reason>"` (npm asks for your security key): the version stays
   installable and prints the reason. A version that is staged but wrong is rejected on npmjs.com (Staged Packages)
   instead of approved.
5. **Homebrew.** In `hslee16/homebrew-klatch`, `git revert` the commit that bumped the formula and open a pull request,
   so the formula points at the previous good release again.

**Who is told.** There are no known users of the pre-releases, so today that is the maintainer. A security problem
goes through a private security advisory (see SECURITY.md), published when it is fixed; the written advisory process
waits for the outside review (#438).

**Not yet exercised.** None of these has been run against a real release. The one piece that has been used is
`gh release delete-asset`, which removed and replaced the assets of the v0.1.0.dev1 test release on 2026-11-14. The
yank, deprecate and revert steps are written from each tool's documented behaviour. Run each once on a throwaway
pre-release before relying on it.

## The dry run

Run by hand: **Actions, Release dry run, Run workflow**, before a release or after changing how one is built. It runs
the whole build half of a release: the same `build-release.yml` that `release.yml` calls, for the tag the version files
name. It builds the Python package and the four platform wheels (and runs `twine check` on them), the npm package
(packed and installed in a clean directory), the crates.io package (verified by building the packaged crate) and the
binaries for all six targets with their SBOMs. It publishes nothing and holds no credential.

It does not run on pull requests: CI is kept light on purpose (see CONTRIBUTING.md).

**It cannot rehearse** the uploads to PyPI, npm and crates.io, the build provenance attestations, the cosign signature
and attaching files to the release: those need the real release and the maintainer, and run only in `release.yml`.
