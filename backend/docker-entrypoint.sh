#!/bin/sh
set -eu

alembic upgrade head

if [ "${SEED_DEMO:-false}" = "true" ]; then
  python backend/scripts/seed.py
fi

exec uvicorn app.main:app \
  --app-dir backend \
  --host 0.0.0.0 \
  --port 8000 \
  --proxy-headers \
  --forwarded-allow-ips="${FORWARDED_ALLOW_IPS:-127.0.0.1}"
