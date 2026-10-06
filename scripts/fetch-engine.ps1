<#
.SYNOPSIS
  Downloads the official Cactus Compute Needle 3 engine for Windows x64 (libneedle3.dll) from Hugging Face,
  pinned to an immutable commit, verifies SHA-256 and extracts the DLL to third_party\needle\.

  Optionally (-WithModel) also fetches whistle.cact for development/testing (the app itself downloads the
  model on first run).
#>
param(
    [switch]$WithModel,
    [switch]$Force
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'third_party\needle'
New-Item -ItemType Directory -Force $out | Out-Null

# Keep these in sync with src/engine.rs and src/model.rs
$EngineRepo    = 'Cactus-Compute/needle3'
$EngineRev     = 'c7c415a3d1b3d929014bc6e866d51ebb971f7089'
$WheelPath     = 'python/cactus_needle-3.1.0-py3-none-win_amd64.whl'
$WheelSha256   = '4f5fc86abfc50d551cdb237a34b501f36d82d4b6f5911ee7bec4e9532d44dd95'
$DllSha256     = 'de2e2c39cd311fbd9971fad4736abc329ed970653674c203e149c4ef27fd1c62'
$ModelRepo     = 'Cactus-Compute/whistle'
$ModelRev      = 'b358ddadd89b7a713b5aa131f23032d3cca1b251'
$ModelSha256   = 'b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb'

function Get-Verified([string]$url, [string]$dest, [string]$sha) {
    if ((Test-Path $dest) -and -not $Force) {
        if ((Get-FileHash $dest -Algorithm SHA256).Hash.ToLower() -eq $sha) { return }
    }
    $tmp = "$dest.part"
    for ($i = 1; $i -le 8; $i++) {
        try { Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing; break }
        catch { if ($i -eq 8) { throw }; Write-Host "  retry $i ($($_.Exception.Message))"; Start-Sleep -Seconds (2 * $i) }
    }
    $got = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLower()
    if ($got -ne $sha) { Remove-Item $tmp -Force; throw "SHA-256 mismatch for $url`n expected $sha`n got      $got" }
    Move-Item $tmp $dest -Force
}

$dll = Join-Path $out 'libneedle3.dll'
if ((Test-Path $dll) -and -not $Force -and (Get-FileHash $dll -Algorithm SHA256).Hash.ToLower() -eq $DllSha256) {
    Write-Host "engine: up to date ($dll)"
} else {
    $wheel = Join-Path $env:TEMP 'whistletype-cactus_needle-3.1.0-win_amd64.whl'
    Write-Host "engine: downloading $EngineRepo@$EngineRev/$WheelPath"
    Get-Verified "https://huggingface.co/$EngineRepo/resolve/$EngineRev/$WheelPath" $wheel $WheelSha256
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($wheel)
    try {
        $entry = $zip.Entries | Where-Object { $_.FullName -eq 'needle/libneedle3.dll' }
        if (-not $entry) { throw 'needle/libneedle3.dll not found in wheel' }
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dll, $true)
    } finally { $zip.Dispose() }
    Remove-Item $wheel -Force
    $got = (Get-FileHash $dll -Algorithm SHA256).Hash.ToLower()
    if ($got -ne $DllSha256) { Remove-Item $dll -Force; throw "libneedle3.dll SHA-256 mismatch: $got" }
    Write-Host "engine: OK $dll"
}

# ---- whisper.cpp CPU runtime pack (ACCURATE engine on the CPU; shipped with the app) -------------------------
$WhisperAsset  = 'https://github.com/ggml-org/whisper.cpp/releases/download/b5130/whisper-bin-x64.zip'
$WhisperSha256 = 'f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c'
$wdir = Join-Path $root 'third_party\whisper-cpu'
$wantDlls = @('whisper.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu-alderlake.dll', 'ggml-cpu-cannonlake.dll', 'ggml-cpu-cascadelake.dll',
    'ggml-cpu-haswell.dll', 'ggml-cpu-icelake.dll', 'ggml-cpu-sandybridge.dll', 'ggml-cpu-skylakex.dll', 'ggml-cpu-sse42.dll', 'ggml-cpu-x64.dll')
if ((Test-Path (Join-Path $wdir 'whisper.dll')) -and -not $Force) {
    Write-Host "whisper-cpu: up to date ($wdir)"
} else {
    New-Item -ItemType Directory -Force $wdir | Out-Null
    $wzip = Join-Path $env:TEMP 'whistletype-whisper-bin-x64.zip'
    Write-Host "whisper-cpu: downloading $WhisperAsset"
    Get-Verified $WhisperAsset $wzip $WhisperSha256
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($wzip)
    try {
        foreach ($n in $wantDlls) {
            $e = $zip.Entries | Where-Object { $_.FullName -eq "Release/$n" }
            if (-not $e) { throw "Release/$n not found in the whisper.cpp archive" }
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($e, (Join-Path $wdir $n), $true)
        }
    } finally { $zip.Dispose() }
    Remove-Item $wzip -Force
    Write-Host "whisper-cpu: OK $wdir"
}
# The prebuilt whisper.cpp DLLs are built with MSVC: deploy the Visual C++ runtime app-locally (allowed by the
# Visual Studio redistribution terms) so they also work on Windows installs without the VC++ redistributable.
$vcFiles = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'vcomp140.dll')
if (-not ($vcFiles | Where-Object { -not (Test-Path (Join-Path $wdir $_)) })) {
    Write-Host 'whisper-cpu: VC++ runtime present'
} else {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $vs = & $vswhere -latest -products * -property installationPath
    $redist = Get-ChildItem (Join-Path $vs 'VC\Redist\MSVC') -Directory | Where-Object { $_.Name -match '^\d' } | Sort-Object Name -Descending | Select-Object -First 1
    if (-not $redist) { throw 'Visual C++ redistributable files not found (install the "Desktop development with C++" workload)' }
    $crt = Join-Path $redist.FullName 'x64\Microsoft.VC143.CRT'
    $omp = Join-Path $redist.FullName 'x64\Microsoft.VC143.OpenMP'
    foreach ($f in 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') { Copy-Item (Join-Path $crt $f) $wdir -Force }
    Copy-Item (Join-Path $omp 'vcomp140.dll') $wdir -Force
    Write-Host "whisper-cpu: VC++ runtime $($redist.Name) copied"
}

if ($WithModel) {
    $model = Join-Path $out 'whistle.cact'
    Write-Host "model: $ModelRepo@$ModelRev/whistle.cact"
    Get-Verified "https://huggingface.co/$ModelRepo/resolve/$ModelRev/whistle.cact" $model $ModelSha256
    Write-Host "model: OK $model"
}
