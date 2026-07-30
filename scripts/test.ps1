$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$env:UV_CACHE_DIR = Join-Path $Root ".uv-cache"
& "$Root\.venv\Scripts\uv.exe" run ruff check backend
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& "$Root\.venv\Scripts\uv.exe" run ruff format --check backend
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& "$Root\.venv\Scripts\uv.exe" run mypy backend
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& "$Root\.venv\Scripts\uv.exe" run pytest
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Set-Location "$Root\frontend"
$Pnpm = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
if ($Pnpm) {
    & $Pnpm.Source lint
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Pnpm.Source typecheck
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Pnpm.Source test
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Pnpm.Source build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} else {
    & npm.cmd run lint
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & npm.cmd run typecheck
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & npm.cmd run test
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
