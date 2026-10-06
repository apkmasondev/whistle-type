<#
.SYNOPSIS
  Builds the production release: tests, optimized exe, staged app folder, portable ZIP and the Inno Setup installer.

  Requires: Rust (x86_64-pc-windows-msvc), Python 3 (for notices), Inno Setup 6 (ISCC.exe) for the installer.
  Output: dist\WhistleType-<ver>-portable-x64.zip, dist\WhistleType-<ver>-setup-x64.exe, dist\SHA256SUMS.txt
#>
param([switch]$SkipTests, [switch]$NoInstaller)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "WhistleType $version"

& "$PSScriptRoot\fetch-engine.ps1"
if (-not $SkipTests) {
    cargo test --release --lib
    if ($LASTEXITCODE -ne 0) { throw 'unit tests failed' }
}
cargo build --release --bin WhistleType
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
python "$PSScriptRoot\gen-notices.py" | Out-Null

$dist = Join-Path $root 'dist'
$app = Join-Path $dist 'app'
if (Test-Path $app) { Remove-Item -Recurse -Force $app }
New-Item -ItemType Directory -Force (Join-Path $app 'LICENSES') | Out-Null
Copy-Item target\release\WhistleType.exe $app
Copy-Item third_party\needle\libneedle3.dll $app
# whisper.cpp CPU runtime for ACCURATE mode (+ app-local VC++ runtime); the CUDA pack is downloaded on demand
New-Item -ItemType Directory -Force (Join-Path $app 'whisper-cpu') | Out-Null
Copy-Item third_party\whisper-cpu\*.dll (Join-Path $app 'whisper-cpu')
Copy-Item LICENSES\* (Join-Path $app 'LICENSES')
Copy-Item THIRD_PARTY_NOTICES.md (Join-Path $app 'THIRD_PARTY_NOTICES.txt')
Copy-Item LICENSE (Join-Path $app 'LICENSE.txt')
@"
WhistleType $version - local push-to-talk dictation for Windows

Hold F8 (default), speak, release: the text is typed where your cursor is.
Speech recognition is performed locally on your computer:
  FAST     - Cactus Compute Whistle on the CPU (downloaded once on first start, 16.9 MB, Apache-2.0)
  ACCURATE - OpenAI Whisper via whisper.cpp, on an NVIDIA GPU when available (Settings > Models..., optional downloads)

Settings: click the WhistleType icon in the notification area.
Data:     %APPDATA%\WhistleType (settings), %LOCALAPPDATA%\WhistleType (model, logs)
Portable: create an empty file named WhistleType.portable next to WhistleType.exe to keep all data in .\data

Licence: MIT (LICENSE.txt). Third-party licences: THIRD_PARTY_NOTICES.txt and the LICENSES folder.
Source code and updates: https://github.com/apkmasondev/whistle-type
"@ | Set-Content (Join-Path $app 'README.txt') -Encoding UTF8

$zip = Join-Path $dist "WhistleType-$version-portable-x64.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $app '*') -DestinationPath $zip

if (-not $NoInstaller) {
    $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw 'Inno Setup 6 not found (winget install JRSoftware.InnoSetup)' }
    & $iscc /Qp "/DAppVersion=$version" installer\WhistleType.iss
    if ($LASTEXITCODE -ne 0) { throw 'installer build failed' }
}

Get-ChildItem $dist -File | ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name } |
    Where-Object { $_ -notmatch 'SHA256SUMS' } | Set-Content (Join-Path $dist 'SHA256SUMS.txt')
Get-ChildItem $dist -File | ForEach-Object { "{0}  {1:N0} bytes" -f $_.Name, $_.Length }
