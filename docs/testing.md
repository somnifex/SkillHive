# Testing and quality gates

The complete local gate mirrors the repository CI:

```powershell
.\scripts\test.ps1
```

Tests use temporary SQLite databases and do not modify `data/skillhive.db`.

## Backend

```powershell
.venv\Scripts\uv.exe run ruff check backend
.venv\Scripts\uv.exe run ruff format --check backend
.venv\Scripts\uv.exe run mypy backend
.venv\Scripts\uv.exe run pytest
```

Backend coverage should include successful behavior, unauthenticated/unauthorized rejection,
owner isolation, group-role boundaries, version behavior, and audit-sensitive workflows.

To validate migrations and idempotent initialization against a disposable database, use the same
steps defined in `.github/workflows/ci.yml`.

## Frontend

```powershell
Set-Location frontend
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Add focused interaction tests for visible behavior and permission-dependent controls. Backend
tests remain required for real authorization because UI hiding is not a security boundary.

## Documentation

Before merging documentation changes:

- verify every relative link;
- check commands against the current scripts and lockfile versions;
- keep English and Simplified Chinese task coverage aligned;
- confirm limitations are not presented as implemented features;
- update `CHANGELOG.md` for user-visible behavior.

## CI failures

Reproduce the exact failed command locally using locked dependencies. Fix the underlying issue;
do not weaken type, lint, migration, or permission checks merely to make the pipeline pass. If the
failure is environment-specific, include versions and a redacted log in the pull request.
