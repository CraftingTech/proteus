# VCS

The version-control conventions this project follows: branches, commits, and the platform.

## Setup

- Main branch: `main`
- Platform: GitHub (`CraftingTech/proteus`)
- Ticketing: GitHub Issues
- CI: `.github/workflows/check.yml` gates PRs with `just check`

## Branches

- Format: not formalized yet (prefer `feat/…`, `fix/…`, `chore/…` when branching)
- Types in use: none yet (repo still pre-first-commit at memory authoring time)

## Commits

- Convention: Conventional Commits preferred (`feat`, `fix`, `chore`, `docs`, `refactor`, `test`)
- Format: `type(scope): description` (scopes like `core`, `crd`, `api`, `controller`)
- Rules: imperative mood; no secrets; AI never commits or pushes unless the user asks

## Commit Strategy

AI should auto commit: `never`
