$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$env:UV_CACHE_DIR = Join-Path $Root ".uv-cache"
& "$Root\.venv\Scripts\uv.exe" run uvicorn app.main:app --app-dir backend --host 127.0.0.1 --port 8000 --reload
