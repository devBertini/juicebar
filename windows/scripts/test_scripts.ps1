# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
# Proves install.ps1, measure.ps1 and uninstall.ps1 inside a temporary folder; touches neither the user's installed
# JuiceBar, his Run value nor %LOCALAPPDATA%\JuiceBar. Use: powershell -NoProfile -File test_scripts.ps1
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('juicebar-scripts-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $tmp | Out-Null
$inst = Join-Path $tmp 'inst'
$data = Join-Path $tmp 'data'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$script:fail = 0
function Check($ok, $what) {
    if ($ok) { Write-Output "PASS $what" } else { Write-Output "FAIL $what"; $script:fail++ }
}
function Run-Script($name, $arguments) {
    $ErrorActionPreference = 'Continue'
    $out = & powershell.exe -NoProfile -File (Join-Path $here $name) @arguments 2>&1 | Out-String
    $code = $LASTEXITCODE
    Write-Output $out
    [pscustomobject]@{ Code = $code; Out = $out }
}
function Listing($folder) {
    if (Test-Path $folder) { (Get-ChildItem $folder -Recurse -Force | ForEach-Object { "$($_.FullName)|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)" }) -join "`n" } else { '<none>' }
}

$ownerData = Join-Path $env:LOCALAPPDATA 'JuiceBar'
$before = Listing $ownerData
$runBefore = (Get-ItemProperty -Path $key -Name JuiceBar -ErrorAction SilentlyContinue).JuiceBar
$throwaway = 'JuiceBarScriptsTest'
$ownerPids = @(Get-Process -Name JuiceBar -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })

try {
    Write-Output '== install'
    $r = Run-Script 'install.ps1' @('-Destination', $inst, '-NoStart', '-NoAutostart')
    Check ($r.Code -eq 0) 'install exits 0'
    Check (Test-Path (Join-Path $inst 'JuiceBar.exe')) 'exe arrived in destination'
    Check ((Get-ItemProperty -Path $key -Name JuiceBar -ErrorAction SilentlyContinue).JuiceBar -eq $runBefore) 'Run value unchanged by -NoAutostart'

    Write-Output '== measure'
    $exe = Join-Path $inst 'JuiceBar.exe'
    $proc = Start-Process -FilePath $exe -ArgumentList @('--data', $data, '--no-hotkey', '--seconds', '40') -PassThru
    Start-Sleep -Seconds 2
    # A test run (--data, --seconds) must not take over a start-with-Windows value: the owner's value, when he has
    # one, and a throwaway value of another name pointing at the installed exe are compared before and after.
    New-ItemProperty -Path $key -Name $throwaway -Value ('"' + $exe + '"') -PropertyType String -Force | Out-Null
    $throwBefore = (Get-ItemProperty -Path $key -Name $throwaway).$throwaway
    Start-Sleep -Seconds 1
    $r = Run-Script 'measure.ps1' @('-ProcessId', $proc.Id, '-Minutes', '0.5', '-Interval', '5')
    Check ($r.Code -eq 0) 'measure exits 0'
    Check ($r.Out -match 'CpuSec' -and $r.Out -match 'Handles') 'table printed'
    Check ($r.Out -match 'CPU: .* % of one core') 'CPU line printed'
    Check ($r.Out -match 'Delta over') 'delta line printed'
    Check ((Get-ItemProperty -Path $key -Name JuiceBar -ErrorAction SilentlyContinue).JuiceBar -eq $runBefore) 'Run value unchanged by a test run (--data)'
    Check ((Get-ItemProperty -Path $key -Name $throwaway -ErrorAction SilentlyContinue).$throwaway -eq $throwBefore) 'throwaway Run value unchanged by a test run'

    Write-Output '== uninstall'
    $r = Run-Script 'uninstall.ps1' @('-Destination', $inst)
    Check ($r.Code -eq 0) 'uninstall exits 0'
    Check (-not (Test-Path $inst)) 'destination folder gone'
    Check (-not (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)) 'installed copy closed'
    Check ((Get-ItemProperty -Path $key -Name JuiceBar -ErrorAction SilentlyContinue).JuiceBar -eq $runBefore) 'Run value unchanged'
    Check ($before -ceq (Listing $ownerData)) 'owner data folder listing unchanged'
    foreach ($id in $ownerPids) { Check ($null -ne (Get-Process -Id $id -ErrorAction SilentlyContinue)) "owner's JuiceBar PID $id still running" }
} finally {
    Remove-ItemProperty -Path $key -Name $throwaway -ErrorAction SilentlyContinue
    Get-Process -Name JuiceBar -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -like "$tmp*" } catch { $false } } | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    if ((Test-Path $tmp) -and $tmp -like '*juicebar-scripts-*') { Remove-Item -LiteralPath $tmp -Recurse -Force }
}
if ($script:fail -eq 0) { Write-Output 'ALL PASSED'; exit 0 } else { Write-Output "$($script:fail) FAILED"; exit 1 }
