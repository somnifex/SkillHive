# Contributing to SkillHive

Thank you for helping improve SkillHive. Contributions of code, tests, documentation, design, and
carefully scoped bug reports are welcome.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Do not report
security vulnerabilities in public issues; use the process in [SECURITY.md](SECURITY.md).

## Before opening an issue

1. Search existing issues to avoid duplicates.
2. Confirm the behavior on the latest `main` branch when practical.
3. Use the provided bug or feature template and include one problem per issue.
4. Remove tokens, passwords, private skill content, and personal data from logs or screenshots.

Questions about installation and operation may already be answered in the
[user guide](makewiki/README.md) or [troubleshooting guide](makewiki/troubleshooting.md).

## Development setup

You need Python 3.12, uv 0.11.28, Node.js 24, pnpm 11.9, and PowerShell.

```powershell
.\scripts\setup.ps1
.\scripts\dev.ps1
```

The setup uses the checked-in lockfiles, creates the local SQLite database, applies Alembic
migrations, and loads idempotent demo data. See [docs/testing.md](docs/testing.md) for individual
quality commands and test-database behavior.

## Making a change

1. Create a short-lived branch from `main`.
2. Keep the change focused; avoid unrelated formatting or dependency updates.
3. Enforce authorization in the backend service layer. Hiding a UI action is not an access

   control.
4. Add tests for the successful path, denied permissions, and tenant/owner isolation where

   relevant.
5. For schema changes, create a new Alembic migration. Never rewrite a migration that may already

   have been applied.
6. Update English and Simplified Chinese user documentation when behavior, configuration, or

   workflows change.
7. Add a concise entry under `Unreleased` in [CHANGELOG.md](CHANGELOG.md) for user-visible changes.

Do not commit `.env`, database files, logs, dependency directories, build output, or real
credentials.

## Quality checks

Run the complete local gate before submitting:

```powershell
.\scripts\test.ps1
```

If a check cannot run in your environment, explain why in the pull request and list the checks
that did run.

## Pull requests

A useful pull request:

- links the issue or explains the user problem;
- describes the chosen behavior and any alternatives considered;
- calls out permission, migration, compatibility, and security impact;
- includes screenshots for visible UI changes;
- lists verification performed;
- remains reviewable and does not mix unrelated refactors.

Maintainers may request changes, split an oversized pull request, or close work that conflicts
with the project direction. Reviews should be technical, specific, and respectful.

## Documentation style

User documentation should tell readers what they can accomplish, what permission they need, and
what result to expect. Keep implementation walkthroughs in `docs/`. Use exact UI labels,
configuration keys, and commands verified against the current source.

## License

By contributing, you agree that your contribution is licensed under the repository's
[MIT License](LICENSE).
