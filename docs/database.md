# Database and migrations

SkillHive uses SQLAlchemy 2 and Alembic. SQLite is the development default; PostgreSQL and MySQL
drivers are installed for shared deployments.

## URLs

```env
DATABASE_URL=sqlite:///./data/skillhive.db
DATABASE_URL=postgresql+psycopg://user:password@host:5432/skillhive
DATABASE_URL=mysql+pymysql://user:password@host:3306/skillhive
```

Use `utf8mb4` for MySQL. Production credentials should belong to a dedicated least-privilege
account and use encrypted transport.

## Apply migrations

```powershell
.venv\Scripts\uv.exe run alembic upgrade head
```

Inspect state:

```powershell
.venv\Scripts\uv.exe run alembic current
.venv\Scripts\uv.exe run alembic heads
```

The container backend applies `upgrade head` before starting the API.

## Create a migration

After changing models:

```powershell
.venv\Scripts\uv.exe run alembic revision --autogenerate -m "describe change"
```

Review generated DDL for all three databases, especially constraints, defaults, indexes, and JSON
behavior. Add data backfills explicitly. Never edit a revision that may already be applied; create
a correcting revision.

## Seed data

```powershell
.venv\Scripts\uv.exe run python backend\scripts\seed.py
```

Seeding is idempotent and intended for development. It creates demo accounts/content and ensures
default personal templates. Set `SEED_DEMO=false` for shared container deployments and never rely
on public demo passwords.

## SQLite files

The default file is `data/skillhive.db` and is ignored by Git. Tests use independent temporary
SQLite databases. Stop writers or take an application-consistent snapshot before copying the
development database.

## Migration review checklist

- Upgrade succeeds from the previous released schema on SQLite.
- PostgreSQL/MySQL types and defaults are compatible.
- Existing rows receive valid non-null values.
- Downtime or locking behavior is documented.
- Seed remains idempotent.
- Tests do not mutate `data/skillhive.db`.