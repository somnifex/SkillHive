$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location "$Root\frontend"
$Pnpm = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
if ($Pnpm) {
    & $Pnpm.Source dev
} else {
    & npm.cmd run dev
}
