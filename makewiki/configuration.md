# Configuration

SkillHive reads settings from environment variables and the root `.env` file. Copy
`.env.example` as a starting point; never commit the resulting `.env`.

## Application and database

| Variable        | Default                         | Purpose                                         |
| --------------- | ------------------------------- | ----------------------------------------------- |
| `APP_NAME`      | `SkillHive`                     | Service name shown by the API                   |
| `ENVIRONMENT`   | `development`                   | Environment label                               |
| `DEBUG`         | `false` in code                 | Enables debug behavior; keep off in production  |
| `API_V1_PREFIX` | `/api/v1`                       | Prefix for version 1 endpoints                  |
| `DATABASE_URL`  | `sqlite:///./data/skillhive.db` | SQLAlchemy database URL                         |
| `SEED_DEMO`     | Compose: `true`                 | Loads development accounts at container startup |

PostgreSQL example:

```env
DATABASE_URL=postgresql+psycopg://skillhive:password@localhost:5432/skillhive
```

MySQL example:

```env
DATABASE_URL=mysql+pymysql://skillhive:password@localhost:3306/skillhive
```

After changing the database URL, apply migrations before starting the API:

```powershell
.venv\Scripts\uv.exe run alembic upgrade head
```

## Authentication and cookies

| Variable                      | Default             | Production guidance                            |
| ----------------------------- | ------------------- | ---------------------------------------------- |
| `JWT_SECRET_KEY`              | development example | Replace with a high-entropy secret             |
| `JWT_ALGORITHM`               | `HS256`             | Change only with a coordinated token migration |
| `ACCESS_TOKEN_EXPIRE_MINUTES` | `15`                | Keep short                                     |
| `REFRESH_TOKEN_EXPIRE_DAYS`   | `7`                 | Match your session policy                      |
| `COOKIE_SECURE`               | `false`             | Set `true` behind HTTPS                        |
| `LOGIN_MAX_ATTEMPTS`          | `5`                 | Maximum failures before temporary lockout      |
| `LOGIN_LOCKOUT_MINUTES`       | `15`                | Lockout duration                               |

Access tokens are held in browser memory. Refresh tokens use an HttpOnly cookie and a revocable
database session. Lockout counters are process-local and reset on backend restart.

## Browser access

`CORS_ORIGINS` is a comma-separated list of exact trusted frontend origins:

```env
CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173
```

For a deployed site, replace these with its HTTPS origin. Do not use a broad wildcard with
credentialed cookies.

## Applying changes

Restart the backend after changing environment settings. Browser-build settings require a
frontend rebuild. Check `/api/v1/health`, then test sign-in and token refresh from the deployed
browser origin.
