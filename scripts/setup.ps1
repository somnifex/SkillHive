$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$Uv = Get-Command uv.exe -ErrorAction SilentlyContinue
if (-not $Uv) {
    $Uv = Get-Command uv -ErrorAction SilentlyContinue
}
if (-not $Uv) {
    throw "uv is required. Install it from https://docs.astral.sh/uv/getting-started/installation/"
}

$Pnpm = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
if (-not $Pnpm) {
    throw "pnpm 11 is required. Install it from https://pnpm.io/installation"
}

if (-not (Test-Path "$Root\.env")) {
    Copy-Item "$Root\.env.example" "$Root\.env"
}

$env:UV_CACHE_DIR = Join-Path $Root ".uv-cache"
& $Uv.Source sync --frozen
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Set-Location "$Root\frontend"
& $Pnpm.Source install --frozen-lockfile
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Set-Location $Root
& "$Root\scripts\init-db.ps1"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "SkillHive setup complete."
