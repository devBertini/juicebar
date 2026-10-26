# Renders JuiceBar design options with headless Chrome and rebuilds the gallery.
#   powershell -NoProfile -File scripts\render_design.ps1 [-Option <n-slug>]
# Output: docs\design\renders\<n-slug>\{mac-light,mac-dark,win-light,win-dark}.png and docs\design\index.html
param(
  [string]$Option = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$repo = Split-Path -Parent $PSScriptRoot
$design = Join-Path $repo 'docs\design'
$optionsDir = Join-Path $design 'options'
$rendersDir = Join-Path $design 'renders'
$chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
if (-not (Test-Path $chrome)) { Write-Error "Chrome not found: $chrome"; exit 1 }

$scenes = @(
  @{ Page = 'mac'; W = 1200; H = 750;  Scale = 2 },
  @{ Page = 'win'; W = 1600; H = 1000; Scale = 1.5 }
)
$themes = @('light', 'dark')
$expectW = 2400; $expectH = 1500

if ($Option) {
  if (-not (Test-Path (Join-Path $optionsDir $Option))) { Write-Error "Option folder not found: $Option"; exit 1 }
  $names = @($Option)
} else {
  $names = @(Get-ChildItem $optionsDir -Directory | Sort-Object Name | ForEach-Object { $_.Name })
}

$profileDir = Join-Path ([System.IO.Path]::GetTempPath()) ('juicebar-chrome-' + [guid]::NewGuid().ToString('N'))
$errFile = Join-Path ([System.IO.Path]::GetTempPath()) ('juicebar-chrome-err-' + [guid]::NewGuid().ToString('N') + '.txt')
$outFile = $errFile + '.out'
$failures = @()
$inv = [System.Globalization.CultureInfo]::InvariantCulture

try {
  foreach ($name in $names) {
    $outDir = Join-Path $rendersDir $name
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    foreach ($s in $scenes) {
      $htmlPath = Join-Path (Join-Path $optionsDir $name) ($s.Page + '.html')
      if (-not (Test-Path $htmlPath)) { $failures += "${name}: missing $($s.Page).html"; continue }
      foreach ($theme in $themes) {
        $png = Join-Path $outDir ("$($s.Page)-$theme.png")
        if (Test-Path $png) { Remove-Item $png -Force }
        $url = ([System.Uri]$htmlPath).AbsoluteUri + '?theme=' + $theme
        $scale = ([double]$s.Scale).ToString($inv)
        $argLine = '--headless=new --disable-gpu --hide-scrollbars --no-first-run --no-default-browser-check ' +
          '--user-data-dir="' + $profileDir + '" --screenshot="' + $png + '" ' +
          "--window-size=$($s.W),$($s.H) --force-device-scale-factor=$scale " +
          '--enable-logging=stderr --v=0 "' + $url + '"'
        $p = Start-Process -FilePath $chrome -ArgumentList $argLine -PassThru -NoNewWindow `
          -RedirectStandardError $errFile -RedirectStandardOutput $outFile
        if (-not $p.WaitForExit(90000)) { try { $p.Kill() } catch {} ; $failures += "${name} $($s.Page)-${theme}: Chrome timed out"; continue }
        $label = "$name $($s.Page)-$theme"
        if (-not (Test-Path $png)) { $failures += "${label}: no PNG written"; continue }
        $img = [System.Drawing.Image]::FromFile($png)
        $w = $img.Width; $h = $img.Height
        $img.Dispose()
        Write-Host ("{0,-28} {1}x{2}" -f $label, $w, $h)
        if ($w -ne $expectW -or $h -ne $expectH) { $failures += "${label}: size ${w}x${h}, expected ${expectW}x${expectH}" }
        $bad = @(Get-Content $errFile -ErrorAction SilentlyContinue | Where-Object { $_ -match 'CONSOLE' -and $_ -match '(?i)Uncaught|error' })
        foreach ($line in $bad) { $failures += "${label}: console: $line" }
      }
    }
  }
}
finally {
  # delete only the throwaway profile and logs this script made in the system temp folder
  $tmp = [System.IO.Path]::GetTempPath()
  if ((Test-Path $profileDir) -and $profileDir.StartsWith($tmp) -and (Split-Path -Leaf $profileDir).StartsWith('juicebar-chrome-')) {
    Start-Sleep -Milliseconds 500
    Remove-Item $profileDir -Recurse -Force -ErrorAction SilentlyContinue
  }
  Remove-Item $errFile, $outFile -Force -ErrorAction SilentlyContinue
}

# ---- gallery: every option that has renders ----
$groups = @()
if (Test-Path $rendersDir) {
  foreach ($d in (Get-ChildItem $rendersDir -Directory | Sort-Object Name)) {
    $files = @('mac-light', 'mac-dark', 'win-light', 'win-dark')
    if (@($files | Where-Object { Test-Path (Join-Path $d.FullName "$_.png") }).Count -eq 0) { continue }
    $title = $d.Name -replace '^(\d+)-(.*)$', 'Op&ccedil;&atilde;o $1: $2'
    $cells = ''
    foreach ($f in $files) {
      $label = @{ 'mac-light' = 'Mac claro'; 'mac-dark' = 'Mac escuro'; 'win-light' = 'Windows claro'; 'win-dark' = 'Windows escuro' }[$f]
      $rel = 'renders/' + $d.Name + '/' + $f + '.png'
      $cells += "<figure><figcaption>$label</figcaption><a href=`"$rel`"><img src=`"$rel`" alt=`"$($d.Name) $label`"></a></figure>`n"
    }
    $groups += "<section><h2>$title</h2><div class=`"grid`">`n$cells</div></section>"
  }
}
$page = @"
<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<title>JuiceBar: op&ccedil;&otilde;es visuais</title>
<style>
  body { margin: 0; padding: 32px; background: #0f1115; color: #e8eaf0; font: 15px/1.4 "Segoe UI Variable Text", "Segoe UI", sans-serif; }
  h1 { font-size: 22px; font-weight: 600; margin: 0 0 28px; }
  section { margin-bottom: 48px; }
  h2 { font-size: 18px; font-weight: 600; margin: 0 0 12px; }
  .grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
  figure { margin: 0; }
  figcaption { font-size: 12px; color: #9aa1b2; margin-bottom: 6px; }
  img { width: 100%; height: auto; display: block; border-radius: 8px; background: #000; }
</style>
</head>
<body>
<h1>JuiceBar: op&ccedil;&otilde;es visuais</h1>
$($groups -join "`n")
</body>
</html>
"@
[System.IO.File]::WriteAllText((Join-Path $design 'index.html'), $page, (New-Object System.Text.UTF8Encoding($false)))

if ($failures.Count -gt 0) {
  Write-Host ''
  Write-Host 'FAILED:' -ForegroundColor Red
  $failures | ForEach-Object { Write-Host "  $_" }
  exit 1
}
Write-Host 'OK'
exit 0
