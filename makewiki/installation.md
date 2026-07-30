# Installation

## Requirements

For the native Windows workflow:

- Windows PowerShell 5.1 or PowerShell 7;
- Python 3.12;
- [uv](https://docs.astral.sh/uv/) 0.11.28;
- Node.js 24;
- pnpm 11.9.

For the container workflow, install Docker with Compose support.

## Local PowerShell installation

From the repository root:

```powershell
.\scripts\setup.ps1
```

The script copies `.env.example` to `.env` when needed, installs dependencies from the lockfiles,
applies all database migrations, and loads idempotent demo data into SQLite.

Start both services:

```powershell
.\scripts\dev.ps1
```

Or run each service in its own terminal:

```powershell
.\scripts\backend.ps1
```

```powershell
.\scripts\frontend.ps1
```

Verify:

- UI: <http://127.0.0.1:5173>
- API health: <http://127.0.0.1:8000/api/v1/health>
- OpenAPI: <http://127.0.0.1:8000/docs>

## Manual local installation

```powershell
uv sync --frozen
Copy-Item .env.example .env
Set-Location frontend
pnpm install --frozen-lockfile
Set-Location ..
.\scripts\init-db.ps1
```

## Docker Compose installation

The default Compose stack uses PostgreSQL:

```powershell
Copy-Item .env.example .env
docker compose up --build
```

It exposes the UI on port `5173` and the API on port `8000`. The backend entry point waits for the
database, applies migrations, and—when `SEED_DEMO=true`—loads demo data.

Stop containers without deleting their volumes:

```powershell
docker compose down
```

## Demo accounts

| Role                   | Username | Password    |
| ---------------------- | -------- | ----------- |
| Platform administrator | `admin`  | `Admin123!` |
| User                   | `howie`  | `User123!`  |
| User                   | `mei`    | `User123!`  |

These accounts are only for local evaluation. Disable demo seeding and rotate or remove every
seed credential before a shared deployment.

Continue with [Configuration](configuration.md) or [Getting started](getting-started.md).
