$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Start-Process powershell -WindowStyle Hidden -ArgumentList "-NoExit", "-ExecutionPolicy", "Bypass", "-File", "$Root\scripts\backend.ps1"
Start-Process powershell -WindowStyle Hidden -ArgumentList "-NoExit", "-ExecutionPolicy", "Bypass", "-File", "$Root\scripts\frontend.ps1"
Write-Host "SkillHive is starting:"
Write-Host "  Frontend: http://127.0.0.1:5173"
Write-Host "  API docs: http://127.0.0.1:8000/docs"
