# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
# Closes the JuiceBar installed in the destination (it restores the device lights on quit), removes the start-with-Windows
# value if it points there, deletes the exe and the folder if empty. With -RemoveData also deletes %LOCALAPPDATA%\JuiceBar.
# Use: powershell -NoProfile -File uninstall.ps1 [-Destination <folder>] [-RemoveData]
param(
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'Programs\JuiceBar'),
    [switch]$RemoveData
)
$ErrorActionPreference = 'Stop'

$Destination = [System.IO.Path]::GetFullPath($Destination)
$target = Join-Path $Destination 'JuiceBar.exe'

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

$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$stored = (Get-ItemProperty -Path $key -Name 'JuiceBar' -ErrorAction SilentlyContinue).JuiceBar
if ($null -ne $stored) {
    if ($stored.Trim('"') -ieq $target) {
        Remove-ItemProperty -Path $key -Name 'JuiceBar'
        Write-Output 'STEP start with Windows value removed'
    } else {
        Write-Output "STEP start with Windows value points elsewhere ($stored), left as it is"
    }
} else {
    Write-Output 'STEP no start with Windows value'
}

if (Test-Path -LiteralPath $target) {
    Remove-Item -LiteralPath $target -Force
    Write-Output "STEP deleted $target"
}
if ((Test-Path -LiteralPath $Destination) -and -not (Get-ChildItem -LiteralPath $Destination -Force)) {
    Remove-Item -LiteralPath $Destination -Force
    Write-Output "STEP removed empty folder $Destination"
}

$data = Join-Path $env:LOCALAPPDATA 'JuiceBar'
if ($RemoveData) {
    if (Test-Path -LiteralPath $data) {
        Write-Output "STEP data folder $data holds:"
        Get-ChildItem -LiteralPath $data -Recurse -Force | ForEach-Object { Write-Output "  $($_.FullName) ($($_.Length) bytes)" }
        Remove-Item -LiteralPath $data -Recurse -Force
        Write-Output 'STEP data folder deleted'
    } else {
        Write-Output 'STEP no data folder'
    }
} else {
    Write-Output "STEP data kept at $data (use -RemoveData to delete)"
}
Write-Output 'DONE'
