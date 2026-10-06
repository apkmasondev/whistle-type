<#
.SYNOPSIS
  Production build in ACCURATE mode: start -> Whisper ready, memory, VRAM and idle CPU with the model loaded.
  -CudaRuntime: a folder with the files of the CUDA 12.4 pack (the "Release" folder of the official zip); without it
  only the CPU case is measured. Writes tests\results\perf-accurate.json. Windows PowerShell 5.1.
#>
param([string]$CudaRuntime = '', [int]$IdleSeconds = 60)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exe = Join-Path $root 'target\release\WhistleType.exe'
$out = [ordered]@{ date = (Get-Date).ToString('s') }

function Vram() { try { [double]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits) -split "`n")[0] } catch { $null } }
function Stats($p) { $p.Refresh(); [ordered]@{ cpu_s = [math]::Round($p.TotalProcessorTime.TotalSeconds, 3); working_set_mb = [math]::Round($p.WorkingSet64 / 1MB, 1); private_mb = [math]::Round($p.PrivateMemorySize64 / 1MB, 1); threads = $p.Threads.Count } }

function Measure-Case($name, $model, $file, [switch]$Gpu) {
    $work = Join-Path $env:TEMP ("wt-perfacc-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $data = Join-Path $work 'data'
    New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0'), (Join-Path $data 'models\whisper') | Out-Null
    Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
    Copy-Item (Join-Path $root "third_party\whisper-models\$file") (Join-Path $data 'models\whisper\')
    if ($Gpu) {
        $pack = Join-Path $data 'runtimes\whisper-cuda-12.4-b5130'
        New-Item -ItemType Directory -Force $pack | Out-Null
        Copy-Item (Join-Path $CudaRuntime '*.dll') $pack
        Copy-Item (Join-Path $root 'third_party\whisper-cpu\*140*.dll') $pack -Force
    }
    Set-Content (Join-Path $data 'settings.json') ('{"engine_mode":"accurate","whisper_model":"' + $model + '"}') -Encoding ASCII
    $log = Join-Path $data 'logs\whistletype.log'
    $vram0 = Vram
    $env:WHISTLETYPE_DATA_DIR = $data
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $exe -ArgumentList '--background' -PassThru
    try {
        $ready = $null
        while ($sw.ElapsedMilliseconds -lt 120000 -and -not $ready) {
            Start-Sleep -Milliseconds 50
            if (Test-Path $log) { $ready = Get-Content $log | Where-Object { $_ -match 'whisper ready|whisper: .* failed' } | Select-Object -First 1 }
        }
        $readyMs = $sw.ElapsedMilliseconds
        Start-Sleep 2
        $a = Stats $p; $vram1 = Vram
        Start-Sleep $IdleSeconds
        $b = Stats $p
        $r = [ordered]@{
            ready_line = $ready; process_start_to_whisper_ready_ms = $readyMs; after_load = $a
            vram_mb = $(if ($vram0 -ne $null -and $vram1 -ne $null) { $vram1 - $vram0 } else { $null })
            idle = [ordered]@{ seconds = $IdleSeconds; cpu_seconds_used = [math]::Round($b.cpu_s - $a.cpu_s, 3); end = $b }
        }
        Write-Host ("{0}: ready {1} ms | ws {2} MB, private {3} MB | VRAM +{4} MB | idle CPU {5} s in {6} s" -f $name, $readyMs, $a.working_set_mb, $a.private_mb, $r.vram_mb, $r.idle.cpu_seconds_used, $IdleSeconds)
        $out[$name] = $r
    } finally {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep 1
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    }
}

if ($CudaRuntime) { Measure-Case 'turbo_gpu' 'whisper-large-v3-turbo' 'ggml-large-v3-turbo-q5_0.bin' -Gpu }
Measure-Case 'small_cpu' 'whisper-small' 'ggml-small-q5_1.bin'
$out | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $root 'tests\results\perf-accurate.json') -Encoding UTF8
