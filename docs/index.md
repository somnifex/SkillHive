# SkillHive developer reference

This directory explains the implementation contracts needed to integrate with or contribute to
SkillHive. Product usage belongs in the bilingual [user guide](../makewiki/README.md).

## References

- [API and authentication](api.md)
- [Architecture and authorization boundaries](architecture.md)
- [Database and migrations](database.md)
- [Testing and quality gates](testing.md)
- [Brand and logo usage](brand.md)

The running backend is the canonical endpoint schema:

- Swagger UI: `http://127.0.0.1:8000/docs`
- ReDoc: `http://127.0.0.1:8000/redoc`
- OpenAPI JSON: `http://127.0.0.1:8000/openapi.json`

## Contributor entry points

Read [CONTRIBUTING.md](../CONTRIBUTING.md) before changing code. Schema changes require a new
Alembic revision, permission decisions belong in backend services/dependencies, and user-visible
behavior must update both English and Simplified Chinese guides.

The project targets Python 3.12, React 19, and three SQL backends: SQLite, PostgreSQL, and MySQL.
