$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$env:UV_CACHE_DIR = Join-Path $Root ".uv-cache"
& "$Root\.venv\Scripts\uv.exe" run --offline alembic upgrade head
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& "$Root\.venv\Scripts\uv.exe" run --offline python backend\scripts\seed.py
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
