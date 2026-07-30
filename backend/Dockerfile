FROM ghcr.io/astral-sh/uv:0.11.28-python3.12-bookworm-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PATH="/app/.venv/bin:$PATH"

WORKDIR /app

COPY pyproject.toml uv.lock .python-version README.md ./
RUN uv sync --frozen --no-dev --no-install-project

COPY alembic.ini ./
COPY backend ./backend

RUN chmod +x /app/backend/docker-entrypoint.sh

EXPOSE 8000
ENTRYPOINT ["/app/backend/docker-entrypoint.sh"]
