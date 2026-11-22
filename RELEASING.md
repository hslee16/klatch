# Releasing

How a release is made. The workflow is `.github/workflows/release.yml`; the rules are in [VERSIONING.md](VERSIONING.md).
Publishing needs the maintainer: the `release` environment approval in GitHub Actions, and the approval of the staged
npm version on npmjs.com with a security key. Nothing here can be done by an agent alone, by design. Which account owns
which registry name, and how a second maintainer takes one over, is in [OWNERS.md](OWNERS.md).

## 1. Pick the number and bump every place it lives

One release is one number, spelled three ways (`0.1.0-dev.2` for the crate and npm, `0.1.0.dev2` for PyPI, tag
`v0.1.0.dev2`). Change all of these in one pull request:

- `proto/Cargo.toml` (the workspace version) and `proto/Cargo.lock` (`cargo build` updates it)
- `clients/python/pyproject.toml` and `clients/python/package/wheel_init.py`
- `clients/typescript/package.json` and `clients/typescript/package-lock.json`
- `CHANGELOG.md`: put the entries under the new version and date: the Unreleased entries, plus what
  `release/changelog-fragments.sh collect` prints (then delete the files in `changelog.d/`)
- `DESIGN.md`: run `release/design-fragments.sh assemble`; it appends the fragments in `design/items/` as the next numbered
  items in ticket order and deletes them. It prints the number each ticket got (keep that output in a file for
  `release/renumber-refs.sh`, below)
- `RESEARCH-NOTES.md`: run `release/research-fragments.sh assemble`; it appends the fragments in `research/notes/` as the
  next numbered sections in ticket order and deletes them. It prints the number each ticket got (keep that output too)
- References: run `release/renumber-refs.sh MAPFILE`, where MAPFILE holds the two assemble outputs (`... 2> map.txt`,
  appended for the second). It rewrites every "design item for #T" to "design item N" and every "research note for #T" to
  "research note section N" across the tree (not release/, target/, node_modules or `*.jsonl`), prints each change, and
  fails without changing anything if a referenced ticket has no item or section. Run it before collecting the changelog
  entries, so those carry the numbers. Afterwards no "design item for #" or "research note for #" with a number remains
- `README.md`: the version in the `cargo install` line and the pre-release notice
- `VERSIONING.md`, if the protocol or store schema version changed

Rehearse the assembly first, on a throwaway copy that never touches the tree: `release/rehearse-assemble.sh` runs the
DESIGN and RESEARCH assemble steps, `release/renumber-refs.sh`, the changelog collect and `check-versions.sh` on a copy,
prints each renumbered reference, the diff stat, the changelog entry count and the "design item for #NNN" and
"research note for #NNN" references that still need a number (none unless a step failed), and removes the copy. It exits non-zero if any step fails.
Add `--check-tests` (#1286) to also run, on the assembled copy, the checks that assembling breaks: the workspace version in
`proto/THIRD-PARTY-LICENSES.txt` (regenerate it with `proto/check-notices.sh --write` at every bump), the `release_scope_docs`
and `negotiate` cargo tests, and the `release/test-*.sh` shell tests. It is slower, so it is off by default. It builds in a fresh
cargo target directory per run (removed afterwards; `REHEARSE_TARGET_DIR` overrides and keeps it), because a shared one reruns a test
binary built for the previous copy (#1292), so expect a full build each time.

Run `release/check-versions.sh`; CI runs it on every pull request, and each registry job repeats the check against the
tag, so a mismatch stops the release before anything is published.

## 2. Merge, tag, approve

1. CI green, merge to `main`.
2. Create a GitHub **pre-release** (while every version is a pre-release) with the tag `v<PyPI spelling>`, from `main`.
   The release body is the output of `release/release-notes.sh <version> > notes.md` (the scope sentence, the macOS
   quarantine note, the supported platforms, the known limits from SECURITY.md "What is not protected" and the open rows
   of the authorization audit, the out-of-scope list, the install and verification commands); pass it with
   `--notes-file notes.md`. Edit `release/RELEASE-NOTES.template.md` to change the fixed text.
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

`release/verify-published.sh [--report FILE] <version>` runs the public-side checks of this list in one go and prints
PASS, FAIL or SKIP for each: PyPI (`pip index versions`), npm (version, `latest`, provenance), crates.io (present, not
yanked), the GitHub release (published, a pre-release, all six archives, six SBOMs, `install.sh`, `SHA256SUMS` and its
signature, through REST), the tap formula on `main`, and a clean install from PyPI into a throwaway virtual environment
that runs `klatch --version` and `klatch-server --help`. It needs no credential. `--report` writes the results as
markdown to paste on the release epic. It does not check the cosign signature or provenance of an archive (use the
SECURITY.md commands on a download) or the tap pull request's CI.

## 4. Homebrew

The formula in `hslee16/homebrew-klatch` (`Formula/klatch.rb`) needs the four download URLs and the four
SHA-256 values from the release's `SHA256SUMS`, plus the `klatch-server` install and test lines. Do not edit it by hand:
download `SHA256SUMS` from the release, check out the tap, and run

```
release/bump-formula.sh --patch bump.patch --pr-body bump-pr.md <version> SHA256SUMS <tap-checkout>
```

The formula has no `version` line: the version comes from the URLs, and `brew audit --strict` (which the tap CI runs) fails a line that
repeats it (#1297). The bump removes the line if there is one.
It rewrites the formula, checks the result (no `version` line, four URLs that carry the tag and the version, four SHA-256 values equal to the
`SHA256SUMS` lines, `klatch-server` installed and tested), prints the patch, and prints the git and `gh api` (REST)
commands that open the tap pull request. It pushes nothing and never calls `gh` or `git`: you run the printed commands
and merge the pull request, whose CI audits, installs and tests the formula on macOS and Linux. It refuses a
`SHA256SUMS` that lacks one of the four archives and leaves the formula as it was.

Rehearse on a Mac before the pull request with `--rehearse <archives-dir>` (the downloaded archives): it points a copy
of the bumped formula at `file://` URLs in a temporary local tap and runs `brew audit --strict --new` (the same audit the tap CI runs),
`brew install --build-from-source` and `brew test`, then removes the tap. It changes your local Homebrew while it runs,
and prints a message and skips when `brew` is missing.

The bump first compares the tap formula with the staged copy `release/homebrew/klatch.rb` (`release/compare-formula.sh`): any
difference beyond the version, URLs and hashes stops it, and after a good bump the staged copy is replaced by the bumped
formula, so commit it with the release. The one allowed difference is the `klatch-server` install and test lines: the staged
copy is the release candidate and may have them while the tap (the previous release) does not yet (`--allow-klatch-server`,
exactly those two lines, nothing else). `release/verify-published.sh` repeats the comparison against the live tap
(`tap-drift`) with no allowance: after the release the tap must carry the same lines as the staged copy.

The staged copy in this repository, `release/homebrew/klatch.rb`, is the file to edit first: `release/check-versions.sh`
checks its four URLs (release tag and archive name, from `release/pep440.sh`, which fix the version) and, when they are
present, its SHA-256 lines, so a version bump cannot leave it behind. After the release exists, check the checksums
too: `release/check-versions.sh . release/homebrew/klatch.rb path/to/SHA256SUMS`. Then copy the file to the tap.

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

**The rehearsal (#576).** `release/rollback-rehearsal.sh <version>` prints, for a version, the exact command of every
step above and runs nothing (the default). `--execute` runs the reversible steps
against a throwaway pre-release such as `0.2.0-alpha.9`: it creates and deletes a draft release and yanks and un-yanks the crate.
It needs `GH_TOKEN` and `CARGO_REGISTRY_TOKEN` in the environment, typed by you at run time and never stored; it refuses without
both, and refuses a version that is not a pre-release. The crates.io token needs the `yank-version` scope even though releases use
trusted publishing, because trusted publishing covers publishing only. npm is by hand (#1295): `npm deprecate` needs a one-time
password, so the script never runs it. It prints the two commands (deprecate, then remove the deprecation) for you to run in a
terminal where you are logged in to npm, and npm asks for the one-time password there; the script never takes a password from the
environment or an argument. The PyPI yank and the tap revert are printed for you to do by hand too (PyPI has no command for a yank).
Revoke the tokens afterwards.

What is permanent, whatever the rehearsal does: the throwaway version number is used up on crates.io, npm and PyPI
(published numbers are never given back, and PyPI file names are never reusable), so rehearse on a number you can spare.
Everything the script runs is reversible apart from that.

## The dry run

Run by hand: **Actions, Release dry run, Run workflow**, before a release or after changing how one is built. It runs
the whole build half of a release: the same `build-release.yml` that `release.yml` calls, for the tag the version files
name. It builds the Python package and the four platform wheels (and runs `twine check` on them), the npm package
(packed and installed in a clean directory), the crates.io package (verified by building the packaged crate) and the
binaries for all six targets with their SBOMs. It publishes nothing and holds no credential.

It does not run on pull requests: CI is kept light on purpose (see CONTRIBUTING.md).

**It cannot rehearse** the uploads to PyPI, npm and crates.io, the build provenance attestations, the cosign signature
and attaching files to the release: those need the real release and the maintainer, and run only in `release.yml`.
