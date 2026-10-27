# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
# Samples a running JuiceBar: CPU seconds, working set, private bytes, handles, GDI and USER objects. Prints a table, the
# deltas and the CPU percentage over the window. Exits 1 when the process ends early or none/many match.
# Use: powershell -NoProfile -File measure.ps1 [-ProcessId <id>] [-Minutes <n>] [-Interval <s>]
param(
    [int]$ProcessId = 0,
    [double]$Minutes = 5,
    [double]$Interval = 5
)
$ErrorActionPreference = 'Stop'

Add-Type -Namespace JB -Name Gui -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetGuiResources(System.IntPtr hProcess, uint uiFlags);
'@

if ($ProcessId -eq 0) {
    $all = @(Get-Process -Name JuiceBar -ErrorAction SilentlyContinue)
    if ($all.Count -ne 1) { Write-Output "FAILED expected one JuiceBar process, found $($all.Count); pass -ProcessId"; exit 1 }
    $ProcessId = $all[0].Id
}

function Sample($id) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($null -eq $p -or $p.HasExited) { return $null }
    try {
        [pscustomobject]@{
            Time    = Get-Date
            CpuSec  = [math]::Round($p.TotalProcessorTime.TotalSeconds, 3)
            WsMB    = [math]::Round($p.WorkingSet64 / 1MB, 2)
            PrivMB  = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
            Handles = $p.HandleCount
            Gdi     = [JB.Gui]::GetGuiResources($p.Handle, 0)
            User    = [JB.Gui]::GetGuiResources($p.Handle, 1)
        }
    } catch { return $null }
}

function Show-Table($rows, $first) {
    $rows | Format-Table @{n = 'T(s)'; e = { [math]::Round(($_.Time - $first.Time).TotalSeconds, 1) } }, CpuSec, WsMB, PrivMB, Handles, Gdi, User -AutoSize | Out-String | Write-Output
}

$rows = @()
$first = Sample $ProcessId
if ($null -eq $first) { Write-Output "FAILED process $ProcessId not running"; exit 1 }
$rows += $first
$end = $first.Time.AddMinutes($Minutes)
while ((Get-Date) -lt $end) {
    $wait = [math]::Min($Interval, ($end - (Get-Date)).TotalSeconds)
    if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]($wait * 1000)) }
    $s = Sample $ProcessId
    if ($null -eq $s) {
        Show-Table $rows $first
        Write-Output "FAILED process $ProcessId ended after $([math]::Round(((Get-Date) - $first.Time).TotalSeconds, 1)) s"
        exit 1
    }
    $rows += $s
}

$last = $rows[-1]
Show-Table $rows $first
$secs = ($last.Time - $first.Time).TotalSeconds
$cpu = $last.CpuSec - $first.CpuSec
$pct = if ($secs -gt 0) { 100 * $cpu / $secs } else { 0 }
$cores = [Environment]::ProcessorCount
Write-Output ('Delta over {0:N1} s: working set {1:N2} MB, private {2:N2} MB, handles {3}, GDI {4}, USER {5}' -f $secs, ($last.WsMB - $first.WsMB), ($last.PrivMB - $first.PrivMB), ($last.Handles - $first.Handles), ($last.Gdi - $first.Gdi), ($last.User - $first.User))
Write-Output ('CPU: {0:N3} s in {1:N1} s = {2:N3} % of one core ({3:N4} % of {4} logical processors)' -f $cpu, $secs, $pct, ($pct / $cores), $cores)
exit 0
