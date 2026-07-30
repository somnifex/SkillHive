# Deployment

The supplied Compose stack is a reproducible evaluation baseline, not a complete production
platform. Operators must provide domain routing, TLS, secret management, monitoring, backups, and
an upgrade policy.

## Choose a database

- **SQLite:** simplest for one local process and evaluation. Back up `data/skillhive.db`; do not

  place it on an unreliable shared filesystem.
- **PostgreSQL:** default Compose choice and the recommended starting point for a shared instance.
- **MySQL:** supported with the PyMySQL driver; use `utf8mb4`.

## Compose baseline

```powershell
Copy-Item .env.example .env
docker compose up --build -d
docker compose ps
```

The frontend proxy forwards `/api/` to the backend. The backend applies Alembic migrations before
starting Uvicorn. Confirm `/api/v1/health` and sign-in before placing traffic on the instance.

An optional MySQL service is available through the `mysql` profile, but changing the backend
database URL is still required:

```powershell
docker compose --profile mysql up mysql
```

## Production hardening

Before exposure to untrusted users:

1. Generate a unique, high-entropy `JWT_SECRET_KEY`.
2. Set `SEED_DEMO=false`; remove or rotate all seeded credentials.
3. Serve only through HTTPS and set `COOKIE_SECURE=true`.
4. Set `CORS_ORIGINS` to exact HTTPS origins.
5. Use a least-privilege database account, encrypted connections, persistent storage, and tested

   backups.
6. Keep `DEBUG=false`; restrict API and database network access.
7. Put request-size, timeout, and rate controls at the reverse proxy.
8. Centralize logs without recording passwords, tokens, cookies, or private skill bodies.
9. Monitor health, failed sign-ins, administrative activity, storage, and backup success.
10. Review the [security policy](../SECURITY.md) and current

    [limitations](faq.md).

For multiple backend replicas, replace the process-local login lockout with shared enforcement or
an edge policy before relying on it as a security control.

## Upgrade procedure

1. Read [CHANGELOG.md](../CHANGELOG.md) and back up the database.
2. Build or pull the new application images.
3. Run `alembic upgrade head` once against the target database.
4. Start the new backend and frontend.
5. Verify health, authentication refresh, a private skill read, and an authorized group workflow.
6. Retain the backup until post-upgrade checks and logs are clean.

Database migrations are designed to move forward. Test restoration and upgrades against a copy of
production data; do not edit an already-applied migration.

## Backups

Back up the database and the deployment's secret configuration separately. A database backup
contains account data, private skill content, templates, sessions, and audit records; encrypt it
and restrict access accordingly. Regularly perform a restore test rather than treating backup
creation alone as proof of recovery.
