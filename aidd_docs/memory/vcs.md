# VCS

The version-control conventions this project follows: branches, commits, and the platform.

## Setup

- Main branch: `main`
- Platform: GitHub (`CraftingTech/proteus`)
- Ticketing: GitHub Issues
- CI: `.github/workflows/check.yml` gates PRs with `just check` + kustomize render; `.github/workflows/image.yml` publishes GHCR images from `main` / tags; `.github/workflows/release.yml` opens a GitHub Release on `v*` tags

## Branches

- Format: not formalized yet (prefer `feat/…`, `fix/…`, `chore/…` when branching)
- Types in use: none yet (repo still pre-first-commit at memory authoring time)

## Releases

- Tag-first: from `main`, push an annotated tag `vX.Y.Z` (prerelease suffixes OK, e.g. `v0.0.1-alpha.2`)
- Do **not** open version-bump PRs or commit overlay/Cargo semver pins for releases
- `image.yml` + `release.yml` run on the tag; install overrides the Deployment image (see `deploy/README.md`)

## Commits

- Convention: Conventional Commits preferred (`feat`, `fix`, `chore`, `docs`, `refactor`, `test`)
- Format: `type(scope): description` (scopes like `core`, `crd`, `api`, `controller`)
- Rules: imperative mood; no secrets; AI never commits or pushes unless the user asks

## Commit Strategy

AI should auto commit: `never`
