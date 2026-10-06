<#
.SYNOPSIS
  End-to-end test of the ACCURATE (Whisper) mode in the real app (test-hooks build, WAV instead of the microphone):

    1. ACCURATE on the CPU (no GPU pack): Whisper small, dictation into Notepad, noise rejected.
    2. GPU pack installed through the Model Manager (Download → confirm). The archive is pre-seeded as a partial
       download, so the app resumes it over the network, verifies the SHA-256, unpacks and verifies every DLL,
       and asks for a restart (the CPU runtime is already loaded).
    3. After the restart: ACCURATE on the GPU (large-v3-turbo), dictation into Notepad, latency.
    4. AUTO uses Whisper on the GPU; FAST does not load Whisper at all.
    5. Deleting the GPU pack while it is in use schedules its removal; it is gone after the next start.

  Needs: third_party\whisper-models\{ggml-small-q5_1.bin, ggml-large-v3-turbo-q5_0.bin} (scripts\fetch-whisper-models.py)
  and the CUDA 12.4 release zip (-CudaZip). Windows PowerShell 5.1. Takes the keyboard focus for ~2 minutes.
#>
param([Parameter(Mandatory = $true)][string]$CudaZip, [int]$ResumeBytes = 4000000)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$exe = Join-Path $root 'target\e2e\release\WhistleType.exe'
$cases = Join-Path $root 'tests\audio\generated\cases'
$work = Join-Path $env:TEMP ("wt-acc-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0'), (Join-Path $data 'models\whisper'), (Join-Path $data 'runtimes') | Out-Null
Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
foreach ($m in 'ggml-small-q5_1.bin', 'ggml-large-v3-turbo-q5_0.bin') { Copy-Item (Join-Path $root "third_party\whisper-models\$m") (Join-Path $data 'models\whisper\') }
$settingsFile = Join-Path $data 'settings.json'
$wavPointer = Join-Path $work 'wav.txt'
$log = Join-Path $data 'logs\whistletype.log'
$packDir = Join-Path $data 'runtimes\whisper-cuda-12.4-b5130'
$results = New-Object System.Collections.ArrayList

Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Text; using System.Threading; using System.Runtime.InteropServices;
public static class K {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=40)] struct IN { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int size);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P cb, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, P cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
  public static uint Pid;
  public static void Key(ushort vk, bool up) { var i = new IN[1]; i[0].type = 1; i[0].ki.vk = vk; i[0].ki.flags = up ? 2u : 0u; SendInput(1, i, Marshal.SizeOf(typeof(IN))); }
  public static void Chord(ushort mod, ushort vk) { Key(mod, false); Key(vk, false); Key(vk, true); Key(mod, true); }
  public static bool Activate(IntPtr h) {
    for (int i = 0; i < 20; i++) { keybd_event(0xE8, 0, 0, UIntPtr.Zero); keybd_event(0xE8, 0, 2, UIntPtr.Zero); ShowWindow(h, 9); SetForegroundWindow(h); Thread.Sleep(150); if (GetForegroundWindow() == h) return true; }
    return false;
  }
  public static IntPtr FindTitle(string part) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ if(!IsWindowVisible(h)) return true; var s=new StringBuilder(512); GetWindowText(h,s,512); if(s.ToString().Contains(part)){f=h;return false;} return true;}, IntPtr.Zero); return f; }
  public static IntPtr Find(string cls) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); uint pid; GetWindowThreadProcessId(h, out pid); if(c.ToString()==cls && IsWindowVisible(h) && pid==Pid){f=h;return false;} return true;}, IntPtr.Zero); return f; }
  public static IntPtr Child(IntPtr parent, string cls) { IntPtr f = IntPtr.Zero; EnumChildWindows(parent, (h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); if(c.ToString()==cls){f=h;return false;} return true;}, IntPtr.Zero); return f; }
}
"@

function Record($name, $ok, $detail) {
    $null = $results.Add([pscustomobject]@{ test = $name; ok = [bool]$ok; detail = $detail })
    Write-Host ("[{0}] {1}: {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail)
}
function Log-Lines() { if (Test-Path $log) { return @(Get-Content $log -Encoding UTF8) } else { return @() } }
function Wait-Log([int]$from, [string]$pattern, [int]$timeoutMs = 20000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $lines = Log-Lines
        if ($lines.Count -gt $from) { $hit = $lines[$from..($lines.Count - 1)] | Where-Object { $_ -match $pattern } | Select-Object -First 1; if ($hit) { return $hit } }
        Start-Sleep -Milliseconds 150
    }
    return $null
}
function Win($cls) { for ($i = 0; $i -lt 80; $i++) { $h = [K]::Find($cls); if ($h -ne [IntPtr]::Zero) { return $h }; Start-Sleep -Milliseconds 100 }; throw "no window $cls" }
function Set-Wav($name) { Set-Content -Path $wavPointer -Value (Join-Path $cases "$name.wav") -Encoding ASCII }
function Wav-Seconds($name) { ((Get-Item (Join-Path $cases "$name.wav")).Length - 44) / 32000.0 }

$script:app = $null
function Start-App([string]$json, [string]$readyPattern, [switch]$Foreground) {
    Set-Content $settingsFile $json -Encoding ASCII
    $from = (Log-Lines).Count
    $args = if ($Foreground) { @() } else { @('--background') }
    $script:app = if ($args.Count) { Start-Process $exe -ArgumentList $args -PassThru } else { Start-Process $exe -PassThru }
    [K]::Pid = [uint32]$script:app.Id
    if ($readyPattern) {
        $line = Wait-Log $from $readyPattern 90000
        return $line
    }
}
function Stop-App() { if ($script:app) { Stop-Process -Id $script:app.Id -Force -ErrorAction SilentlyContinue; $script:app.WaitForExit(5000) | Out-Null; $script:app = $null; Start-Sleep 1 } }

# Holds F8 for the clip (+ margin) in Notepad and waits for the result line.
function Dictate([IntPtr]$target, [string]$wav, [string]$expect = 'inserted (Pasted|Typed)') {
    Set-Wav $wav
    if (-not [K]::Activate($target)) { throw 'could not focus Notepad' }
    $from = (Log-Lines).Count
    [K]::Key(0x77, $false); Start-Sleep -Milliseconds ([int]((Wav-Seconds $wav) * 1000) + 300); [K]::Key(0x77, $true)
    $line = Wait-Log $from $expect 60000
    $engine = Wait-Log $from 'dictation #\d+: .* on (CPU|GPU) \(' 1000
    return [pscustomobject]@{ line = $line; engine = $engine; latency = $(if ($line -match 'release->inserted (\d+) ms') { [int]$Matches[1] } else { $null }) }
}
function Notepad-Text($file, $h) { [K]::Activate($h) | Out-Null; [K]::Chord(0x11, 0x53); Start-Sleep 1; Get-Content $file -Raw -Encoding UTF8 }

$env:WHISTLETYPE_DATA_DIR = $data
$env:WHISTLETYPE_TEST_WAV = $wavPointer
Set-Wav 'sent__pl_basic__paulina_normal'
$file = Join-Path $work 'accurate.txt'; Set-Content $file '' -NoNewline
Start-Process notepad.exe -ArgumentList "`"$file`"" | Out-Null
for ($i = 0; $i -lt 50 -and ($np = [K]::FindTitle('accurate.txt')) -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 200 }
if ($np -eq [IntPtr]::Zero) { throw 'Notepad did not open' }
try {
    # 1. ACCURATE on the CPU -------------------------------------------------------------------------------------
    $ready = Start-App '{"ui_language":"en","engine_mode":"accurate","whisper_model":"whisper-small"}' 'whisper ready|whisper: .* failed'
    Record 'CPU: Whisper small loads on the CPU without the GPU pack' ($ready -match 'whisper ready: Whisper small on Cpu') $ready
    $before = Notepad-Text $file $np
    $r = Dictate $np 'sent__pl_mixed__paulina_normal'
    $text = (Notepad-Text $file $np).Substring($before.Length)
    Record 'CPU: dictation transcribed by Whisper on the CPU' (($r.engine -match 'Whisper small on CPU \(accurate\)') -and $text.Trim().Length -gt 10) ("'" + $text.Trim() + "' latency $($r.latency) ms")
    $before = Notepad-Text $file $np
    $r = Dictate $np 'ns_keyboard_typing' '(-> (Silence|NoSpeech|TooShort))|inserted'
    Record 'CPU: keyboard noise inserts nothing' (((Notepad-Text $file $np) -eq $before) -and ($r.line -notmatch 'inserted')) $r.line

    # 2. GPU pack through the Model Manager (resumed download) --------------------------------------------------------
    Stop-App
    $part = Join-Path $data 'runtimes\whisper-cuda-12.4-b5130.zip.part'
    $src = [IO.File]::OpenRead($CudaZip); $dst = [IO.File]::Create($part)
    try { $buf = New-Object byte[] (1MB); $left = $src.Length - $ResumeBytes; while ($left -gt 0) { $n = $src.Read($buf, 0, [Math]::Min($buf.Length, $left)); $dst.Write($buf, 0, $n); $left -= $n } } finally { $src.Close(); $dst.Close() }
    $ready = Start-App '{"ui_language":"en","engine_mode":"accurate","whisper_model":"whisper-small"}' 'whisper ready' -Foreground
    $s = Win 'WhistleType.Settings'
    [K]::SendMessage($s, 0x0111, [IntPtr]127, [K]::GetDlgItem($s, 127)) | Out-Null                # Models…
    $m = Win 'WhistleType.Models'
    $lv = [K]::Child($m, 'SysListView32')
    [K]::PostMessage($lv, 0x0100, [IntPtr]0x23, [IntPtr]::Zero) | Out-Null                         # WM_KEYDOWN End -> GPU pack row
    Start-Sleep -Milliseconds 600
    $from = (Log-Lines).Count
    [K]::PostMessage([K]::GetDlgItem($m, 404), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null   # Download
    $dlg = [IntPtr]::Zero; for ($i = 0; $i -lt 40 -and $dlg -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $dlg = [K]::Find('#32770') }
    Record 'GPU pack: Download asks for confirmation' ($dlg -ne [IntPtr]::Zero) ''
    [K]::PostMessage($dlg, 0x0111, [IntPtr]1, [IntPtr]::Zero) | Out-Null                             # IDOK
    $resumed = Wait-Log $from 'download: resuming at' 30000
    $done = Wait-Log $from 'models: CUDA runtime pack installed|download: (failed|error)' 600000
    $installed = Wait-Log $from 'models: CUDA runtime pack installed' 600000
    Record 'GPU pack: partial download resumed over the network' ($resumed -match ('resuming at ' + ((Get-Item $CudaZip).Length - $ResumeBytes))) $resumed
    Record 'GPU pack: verified and installed by the app' (($installed -ne $null) -and (Test-Path (Join-Path $packDir 'ggml-cuda.dll')) -and -not (Test-Path $part)) $installed
    for ($i = 0; $i -lt 30 -and @(Get-ChildItem (Join-Path $data 'runtimes') -Filter '*.zip*').Count -gt 0; $i++) { Start-Sleep -Milliseconds 100 }
    $zipLeft = @(Get-ChildItem (Join-Path $data 'runtimes') -Filter '*.zip*').Count
    Record 'GPU pack: archive removed after unpacking' ($zipLeft -eq 0) "$zipLeft archive files left"

    # 3. ACCURATE on the GPU after a restart ---------------------------------------------------------------------
    Stop-App
    $ready = Start-App '{"ui_language":"en","engine_mode":"accurate","whisper_model":"whisper-large-v3-turbo"}' 'whisper ready|whisper: .* failed'
    Record 'GPU: large-v3-turbo loads on the GPU after the restart' ($ready -match 'whisper ready: Whisper large-v3-turbo on Gpu') $ready
    $lat = @()
    foreach ($wav in 'sent__pl_mixed__paulina_normal', 'sent__pl_story__paulina_normal', 'sent__pl_tech__paulina_normal') {
        $before = Notepad-Text $file $np
        $r = Dictate $np $wav
        $text = (Notepad-Text $file $np).Substring($before.Length)
        $lat += $r.latency
        Record "GPU: $wav" (($r.engine -match 'Whisper large-v3-turbo on GPU \(accurate\)') -and $text.Trim().Length -gt 10) ("'" + $text.Trim() + "' latency $($r.latency) ms")
    }
    $before = Notepad-Text $file $np
    $r = Dictate $np 'ns_white_noise' '(-> (Silence|NoSpeech|TooShort))|inserted'
    Record 'GPU: white noise inserts nothing' (((Notepad-Text $file $np) -eq $before) -and ($r.line -notmatch 'inserted')) $r.line

    # 4. AUTO and FAST ------------------------------------------------------------------------------------------
    Stop-App
    $ready = Start-App '{"ui_language":"en","engine_mode":"auto","whisper_model":"whisper-large-v3-turbo"}' 'whisper ready|whisper: .* failed'
    $r = Dictate $np 'sent__pl_basic__paulina_normal'
    Record 'AUTO: uses Whisper on the GPU' ($r.engine -match 'on GPU \(accurate\)') $r.engine
    Stop-App
    $from = (Log-Lines).Count
    $ready = Start-App '{"ui_language":"en","engine_mode":"fast","whisper_model":"whisper-large-v3-turbo"}' 'model ready'
    $r = Dictate $np 'sent__pl_basic__paulina_normal'
    $whisperLoaded = Wait-Log $from 'whisper ready' 500
    Record 'FAST: Whistle only, Whisper not loaded' (($r.engine -match 'Whistle .* on CPU \(fast\)') -and -not $whisperLoaded) $r.engine

    # 4b. AUTO with the GPU pack but no usable GPU (CUDA hidden): Whisper must not be loaded on the CPU
    Stop-App
    $env:CUDA_VISIBLE_DEVICES = '-1'
    $from = (Log-Lines).Count
    $ready = Start-App '{"ui_language":"en","engine_mode":"auto","whisper_model":"whisper-large-v3-turbo"}' 'no usable GPU|whisper ready'
    $r = Dictate $np 'sent__pl_basic__paulina_normal'
    $cpuLoad = Wait-Log $from 'whisper ready' 500
    Record 'AUTO without a usable GPU: Whisper not loaded on the CPU, Whistle dictates' (($ready -match 'no usable GPU') -and -not $cpuLoad -and ($r.engine -match 'Whistle .* on CPU \(fast\)')) ("$ready | $($r.engine)")
    Remove-Item Env:CUDA_VISIBLE_DEVICES

    # 5. delete the GPU pack while it is in use -> removed at the next start ---------------------------------------
    Stop-App
    $ready = Start-App '{"ui_language":"en","engine_mode":"accurate","whisper_model":"whisper-large-v3-turbo"}' 'whisper ready' -Foreground
    $s = Win 'WhistleType.Settings'
    [K]::SendMessage($s, 0x0111, [IntPtr]127, [K]::GetDlgItem($s, 127)) | Out-Null
    $m = Win 'WhistleType.Models'
    [K]::PostMessage([K]::Child($m, 'SysListView32'), 0x0100, [IntPtr]0x23, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 600
    [K]::PostMessage([K]::GetDlgItem($m, 405), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null   # Delete
    $dlg = [IntPtr]::Zero; for ($i = 0; $i -lt 40 -and $dlg -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $dlg = [K]::Find('#32770') }
    [K]::PostMessage($dlg, 0x0111, [IntPtr]1, [IntPtr]::Zero) | Out-Null
    for ($i = 0; $i -lt 30 -and -not (Test-Path ($packDir + '.remove')); $i++) { Start-Sleep -Milliseconds 100 }
    Record 'GPU pack in use: removal scheduled' (Test-Path ($packDir + '.remove')) ''
    Stop-App
    $ready = Start-App '{"ui_language":"en","engine_mode":"accurate","whisper_model":"whisper-large-v3-turbo"}' 'whisper ready|whisper: .* failed'
    Record 'GPU pack removed at the next start, Whisper falls back to the CPU runtime' ((-not (Test-Path $packDir)) -and ($ready -match 'on Cpu')) $ready
}
finally {
    Stop-App
    if ($np -ne [IntPtr]::Zero -and [K]::Activate($np)) { [K]::Chord(0x11, 0x53); Start-Sleep -Milliseconds 400; [K]::Chord(0x11, 0x57) }
    $errs = @(Log-Lines | Where-Object { $_ -match '\[ERROR\]|panic' })
    Record 'no errors in the log' ($errs.Count -eq 0) ($errs -join ' | ')
    $resDir = Join-Path $root 'tests\results'
    $results | ConvertTo-Json | Set-Content (Join-Path $resDir 'e2e-accurate.json') -Encoding UTF8
    Copy-Item $log (Join-Path $resDir 'e2e-accurate-app.log') -ErrorAction SilentlyContinue
    Write-Host ("{0} / {1} passed" -f @($results | Where-Object ok).Count, $results.Count)
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
