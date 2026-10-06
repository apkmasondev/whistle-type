<#
.SYNOPSIS
  Measures WhistleType performance and writes tests\results\perf.json.

  1. Production build (target\release): start-up time, model load, idle CPU / RAM / threads.
  2. test-hooks build (target\e2e\release, same code + WAV audio source): latency from hotkey release to text
     in Notepad for 1-29 s clips, CPU used while transcribing, peak RAM/threads.

  Run with Windows PowerShell 5.1. Takes ~4 minutes and the keyboard focus during part 2.
#>
param([int]$IdleSeconds = 60)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$cases = Join-Path $root 'tests\audio\generated\cases'
$work = Join-Path $env:TEMP ("wt-perf-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0') | Out-Null
Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
$log = Join-Path $data 'logs\whistletype.log'
$result = [ordered]@{ machine = (Get-CimInstance Win32_Processor).Name; logical_cpus = [Environment]::ProcessorCount; date = (Get-Date).ToString('s') }

Add-Type @"
using System; using System.Threading; using System.Runtime.InteropServices;
public static class K {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=40)] struct IN { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int size);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  public static void Key(ushort vk, bool up) { var i = new IN[1]; i[0].type = 1; i[0].ki.vk = vk; i[0].ki.flags = up ? 2u : 0u; SendInput(1, i, Marshal.SizeOf(typeof(IN))); }
  public static bool Activate(IntPtr h) { for (int i = 0; i < 20; i++) { keybd_event(0xE8,0,0,UIntPtr.Zero); keybd_event(0xE8,0,2,UIntPtr.Zero); SetForegroundWindow(h); Thread.Sleep(150); if (GetForegroundWindow() == h) return true; } return false; }
}
"@

function Log-Lines() { if (Test-Path $log) { @(Get-Content $log -Encoding UTF8) } else { @() } }
function Wait-Log([int]$from, [string]$pattern, [int]$timeoutMs = 30000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $l = Log-Lines
        if ($l.Count -gt $from) { $hit = $l[$from..($l.Count - 1)] | Where-Object { $_ -match $pattern } | Select-Object -First 1; if ($hit) { return $hit } }
        Start-Sleep -Milliseconds 50
    }
    return $null
}
function Snapshot($p) {
    $p.Refresh()
    [ordered]@{ cpu_s = [math]::Round($p.TotalProcessorTime.TotalSeconds, 3); working_set_mb = [math]::Round($p.WorkingSet64 / 1MB, 1); private_mb = [math]::Round($p.PrivateMemorySize64 / 1MB, 1); threads = $p.Threads.Count; handles = $p.HandleCount }
}
function Idle($p, [int]$secs) {
    $a = Snapshot $p; Start-Sleep -Seconds $secs; $b = Snapshot $p
    [ordered]@{ seconds = $secs; cpu_seconds_used = [math]::Round($b.cpu_s - $a.cpu_s, 4); cpu_percent_of_one_core = [math]::Round(100 * ($b.cpu_s - $a.cpu_s) / $secs, 3); end = $b }
}

$env:WHISTLETYPE_DATA_DIR = $data

# ---- 1. production build ---------------------------------------------------------------------------------
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process (Join-Path $root 'target\release\WhistleType.exe') -ArgumentList '--background' -PassThru
$started = Wait-Log 0 'started in'
$ready = Wait-Log 0 'model ready'
$readyMs = $sw.ElapsedMilliseconds
$result.production = [ordered]@{
    exe_size_kb = [math]::Round((Get-Item (Join-Path $root 'target\release\WhistleType.exe')).Length / 1KB)
    process_start_to_model_ready_ms = $readyMs
    log_started = [string]$started
    log_model = [string](Log-Lines | Where-Object { $_ -match "model loaded in" } | Select-Object -First 1)
    after_start = (Snapshot $p)
}
Start-Sleep 5
$result.production.idle = Idle $p $IdleSeconds
Stop-Process -Id $p.Id -Force
Start-Sleep 1

# ---- 2. dictation latency (test-hooks build) -----------------------------------------------------------
$pointer = Join-Path $work 'wav.txt'
$env:WHISTLETYPE_TEST_WAV = $pointer
Set-Content $pointer (Join-Path $cases 'len_05s.wav')
$p = Start-Process (Join-Path $root 'target\e2e\release\WhistleType.exe') -ArgumentList '--background' -PassThru
$from0 = (Log-Lines).Count
if (-not (Wait-Log $from0 'model ready')) { throw 'model not ready' }
$file = Join-Path $work 'perf.txt'; Set-Content $file '' -NoNewline
Start-Process notepad.exe -ArgumentList "`"$file`""
Start-Sleep 3
$np = (Get-Process notepad | Where-Object { $_.MainWindowTitle -like '*perf.txt*' } | Select-Object -First 1)
$h = if ($np) { $np.MainWindowHandle } else { [K]::GetForegroundWindow() }
[K]::Activate($h) | Out-Null; Start-Sleep 1; [K]::Key(0x23, $false); [K]::Key(0x23, $true)

$runs = @()
foreach ($clip in @('len_01s', 'len_05s', 'len_05s', 'len_05s', 'len_10s', 'len_20s', 'len_29s', 'sent__pl_mixed__paulina_normal', 'sent__pl_story__adam_normal')) {
    Set-Content $pointer (Join-Path $cases "$clip.wav")
    $secs = ((Get-Item (Join-Path $cases "$clip.wav")).Length - 44) / 32000.0
    if (-not [K]::Activate($h)) { Write-Host "focus lost, skipping $clip"; continue }
    $from = (Log-Lines).Count
    [K]::Key(0x77, $false)
    $peakThreads = 0
    $t = [Diagnostics.Stopwatch]::StartNew()
    while ($t.Elapsed.TotalSeconds -lt $secs + 0.2) { Start-Sleep -Milliseconds 50 }
    $before = Snapshot $p
    [K]::Key(0x77, $true)
    $released = [Diagnostics.Stopwatch]::StartNew()
    $peakWs = 0
    $line = $null
    while (-not $line -and $released.ElapsedMilliseconds -lt 30000) {
        $p.Refresh(); $peakThreads = [math]::Max($peakThreads, $p.Threads.Count); $peakWs = [math]::Max($peakWs, $p.WorkingSet64)
        $line = Wait-Log $from 'inserted' 30
    }
    $after = Snapshot $p
    $text = Wait-Log $from 'release->text' 10
    $read = Wait-Log $from 'target read the text' 10
    $runs += [ordered]@{
        clip = $clip; audio_s = [math]::Round($secs, 2)
        release_to_text_ms = if ($text -match 'release->text (\d+) ms') { [int]$Matches[1] } else { $null }
        engine_ms = if ($text -match 'engine (\d+) ms') { [int]$Matches[1] } else { $null }
        paste_read_after_keystroke_ms = if ($read -match 'after the paste keystroke' -and $read -match 'text (\d+) ms') { [int]$Matches[1] } else { $null }
        release_to_inserted_incl_clipboard_restore_ms = if ($line -match 'release->inserted (\d+) ms') { [int]$Matches[1] } else { $null }
        cpu_seconds_used = [math]::Round($after.cpu_s - $before.cpu_s, 3)
        peak_threads = $peakThreads
        peak_working_set_mb = [math]::Round($peakWs / 1MB, 1)
    }
    Write-Host ($runs[-1] | ConvertTo-Json -Compress)
    Start-Sleep -Milliseconds 800
}
$result.dictation = $runs
$result.after_dictation = Snapshot $p
Start-Sleep 5
$result.idle_after_dictation = Idle $p 30
Stop-Process -Id $p.Id -Force
[K]::Activate($h) | Out-Null; [K]::Key(0x11, $false); [K]::Key(0x53, $false); [K]::Key(0x53, $true); [K]::Key(0x57, $false); [K]::Key(0x57, $true); [K]::Key(0x11, $true)

$out = Join-Path $root 'tests\results'
New-Item -ItemType Directory -Force $out | Out-Null
$result | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $out 'perf.json') -Encoding UTF8
Write-Host "written $out\perf.json"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
