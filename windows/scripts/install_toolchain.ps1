# Installs the Windows build toolchain of JuiceBar: Visual Studio Build Tools with the C++ workload (MSVC linker and
# Windows SDK) and rustup with the stable MSVC Rust toolchain, both from winget, newest versions.
# The Build Tools installer asks for elevation once (UAC). Prints one "STEP" line per phase and "DONE" or "FAILED".
$ErrorActionPreference = 'Continue'

function Step($text) { Write-Output "STEP $text" }

Step 'Visual Studio Build Tools (C++ workload)'
winget install --id Microsoft.VisualStudio.BuildTools -e --source winget --accept-package-agreements --accept-source-agreements --override "--quiet --wait --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
Write-Output "STEP buildtools exit $LASTEXITCODE"

Step 'rustup'
winget install --id Rustlang.Rustup -e --source winget --accept-package-agreements --accept-source-agreements
Write-Output "STEP rustup exit $LASTEXITCODE"

$rustup = Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'
if (-not (Test-Path $rustup)) { Write-Output 'FAILED rustup.exe not found'; exit 1 }
& $rustup set profile minimal
& $rustup toolchain install stable --profile minimal --component clippy rustfmt
& $rustup default stable
& (Join-Path $env:USERPROFILE '.cargo\bin\rustc.exe') --version
& (Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe') --version

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName
    & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationVersion
} else { Write-Output 'FAILED vswhere not found'; exit 1 }
Write-Output 'DONE'
