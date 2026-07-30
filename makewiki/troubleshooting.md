# Troubleshooting

## The UI cannot reach the API

1. Open <http://127.0.0.1:8000/api/v1/health>.
2. Confirm the backend process is running on port `8000`.
3. Check that the browser origin appears exactly in `CORS_ORIGINS`.
4. If using containers, inspect `docker compose ps` and backend logs.
5. Rebuild the frontend after changing build-time routing or proxy settings.

Avoid mixing `localhost` and `127.0.0.1` when cookies or CORS are configured for only one.

## Sign-in succeeds, then immediately expires

Check system time, `JWT_SECRET_KEY` consistency across backend restarts/replicas, cookie domain
and path, and browser acceptance of the refresh cookie. Under HTTPS, set `COOKIE_SECURE=true`; on
plain local HTTP it must remain `false`.

## The account is temporarily locked

Wait for `LOGIN_LOCKOUT_MINUTES` (15 by default) and verify the credentials. Restarting the
backend clears the current in-memory counter, but that is a development diagnostic—not an
appropriate production recovery process.

## Migrations fail

Confirm `DATABASE_URL` points to a reachable database and the account can create/alter the
application schema. Then run:

```powershell
.venv\Scripts\uv.exe run alembic current
.venv\Scripts\uv.exe run alembic upgrade head
```

Do not delete migration history or edit an applied migration to force success. Back up the
database and investigate the first failing revision.

## The SQLite database is locked

Stop duplicate backend processes and tools holding the file open. SQLite is not intended for many
concurrent writers; move a shared instance to PostgreSQL rather than repeatedly increasing
timeouts.

## A template or group skill is missing

For a template, check its scope and your current group membership/role. For a group skill, confirm
the platform administrator published and granted it, then confirm group management enabled it.
Disabled or archived global skills are not usable.

## PowerShell blocks a script

Review the script first, then run it in a PowerShell session whose execution policy permits local
scripts. Prefer a process-scoped policy change approved by your organization rather than weakening
the machine-wide policy.

## Dependency installation differs from CI

Use the locked commands: `uv sync --frozen` and `pnpm install --frozen-lockfile`. Confirm Python
3.12, Node.js 24, pnpm 11.9, and uv 0.11.28.

## Need more evidence?

Run the [quality checks](../docs/testing.md), record the exact failing command and version, redact
secrets/private content, then open a bug report using the repository template.
