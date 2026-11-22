# Owners

Which account owns which registry name. Facts from the owner on #417 (2026-11-22). No secrets, codes or email addresses
belong in this file.

| Name | Where | Owner |
|------|-------|-------|
| `klatch` | PyPI (https://pypi.org/project/klatch/) | Alex Lee |
| `@klatch` (the npm org; package `@klatch/client`) | npm | Alex Lee |
| `klatch` | crates.io (`cargo owner --list klatch` shows `hslee16`) | Alex Lee |
| `hslee16/homebrew-klatch` (the Homebrew tap) | GitHub | Alex Lee |
| `hslee16/klatch` (this repository) | GitHub | Alex Lee |

One person owns every account, so the bus factor is one (#417). Recovery codes for the accounts are kept outside this
repository, by the owner; this file does not say where, and never will hold them.

## How each registry is published (from `.github/workflows/release.yml` and RELEASING.md)

- **PyPI:** trusted publishing (OIDC, no stored token), from `release.yml`, in the `release` environment of this repository.
- **crates.io:** trusted publishing from `release.yml`, in the `release` environment, through a short-lived token that the
  job gets and revokes. Whether crates.io is set to require trusted publishing for every new version is a setting on
  crates.io that this repository does not record.
- **npm:** trusted publishing from `release.yml` in the `release` environment, set to stage only. The owner approves the
  staged version on npmjs.com with a security key; CI alone cannot make a version public.
- **Homebrew tap:** a pull request to `hslee16/homebrew-klatch`, made by hand with `release/bump-formula.sh`.
- **GitHub:** the owner approves the `release` environment in Actions.

## Rights that need the owner

- **Yank (crates.io):** an API token with the `yank-version` scope (#1295); trusted publishing cannot yank.
- **Yank (PyPI), deprecate or reject (npm), revert (tap):** the owner's own account (RELEASING.md, rollback section). npm
  asks for the security key.
- **Approvals:** the `release` environment (GitHub) and staged npm versions (security key).

## What a second maintainer needs to take over a registry

A pointer list; the owner does each step, and nothing here is a credential.

- **GitHub:** add them as a repository admin and as a required reviewer of the `release` environment (Settings,
  Environments); give them write access to `hslee16/homebrew-klatch`.
- **PyPI:** add them as a Maintainer or Owner of the `klatch` project (project Collaborators page). The trusted publisher
  entry (this repository, `release.yml`, environment `release`) stays as it is.
- **npm:** invite them to the `klatch` org on npmjs.com, with a security key of their own; they approve staged versions
  under Staged Packages.
- **crates.io:** `cargo owner --add <github-user> klatch`; they need their own `yank-version` token to yank.
- **Both of you:** keep your own recovery codes outside the repository, then update the table above in a pull request and
  confirm it on #417.
