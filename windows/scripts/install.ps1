# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the release of JuiceBar, closes the copy installed in the destination (only that one), copies JuiceBar.exe
# there, sets the start-with-Windows value like the app's menu does, and starts it. Never touches %LOCALAPPDATA%\JuiceBar.
# Use: powershell -NoProfile -File install.ps1 [-Destination <folder>] [-NoStart] [-NoAutostart]
param(
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'Programs\JuiceBar'),
    [switch]$NoStart,
    [switch]$NoAutostart
)
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$manifest = Join-Path $repo 'windows\Cargo.toml'
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) { Write-Output "FAILED cargo not found at $cargo"; exit 1 }

$Destination = [System.IO.Path]::GetFullPath($Destination)
$target = Join-Path $Destination 'JuiceBar.exe'

Write-Output 'STEP build release'
$ErrorActionPreference = 'Continue'
& $cargo build --release --manifest-path $manifest -p juicebar
$buildCode = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($buildCode -ne 0) { Write-Output "FAILED build exit $buildCode"; exit 1 }
$built = Join-Path $repo 'windows\target\release\JuiceBar.exe'
if (-not (Test-Path $built)) { Write-Output "FAILED $built not found"; exit 1 }

# Only a process whose exe is exactly the installed one is closed; any other JuiceBar is left alone.
function Get-Installed($path) {
    @(Get-Process -Name JuiceBar -ErrorAction SilentlyContinue | Where-Object {
        try { $_.Path -ieq $path } catch { $false }
    })
}

$running = Get-Installed $target
if ($running.Count -gt 0) {
    Write-Output "STEP closing $($running.Count) running JuiceBar from $target"
    foreach ($p in $running) { & taskkill.exe /PID $p.Id | Out-Null }
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Installed $target).Count -gt 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
    foreach ($p in (Get-Installed $target)) {
        Write-Output "STEP did not close in 10 s, forcing PID $($p.Id) (device lights may stay as they were)"
        & taskkill.exe /F /PID $p.Id | Out-Null
    }
    Start-Sleep -Milliseconds 500
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null
Copy-Item -LiteralPath $built -Destination $target -Force
Write-Output "STEP copied to $target"

if (-not $NoAutostart) {
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    Set-ItemProperty -Path $key -Name 'JuiceBar' -Value ('"' + $target + '"') -Type String
    Write-Output "STEP start with Windows set: $((Get-ItemProperty -Path $key -Name JuiceBar).JuiceBar)"
} else {
    Write-Output 'STEP start with Windows left untouched'
}

if (-not $NoStart) {
    Start-Process -FilePath $target
    Write-Output 'STEP started'
}
Write-Output 'DONE'
