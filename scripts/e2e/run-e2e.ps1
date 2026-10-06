<#
.SYNOPSIS
  End-to-end test of WhistleType against real Windows applications.

  Uses the `test-hooks` build (target\e2e\release\WhistleType.exe): the microphone is replaced by a WAV file
  played in real time. Everything else is the production code path (F8 injected by the test is accepted like
  a key from any other software; only WhistleType's own tagged keystrokes are ignored):
  keyboard hook → WASAPI-equivalent capture thread → speech gate → Whistle → clipboard/SendInput → target app.

  Run with Windows PowerShell 5.1 (UI Automation / WinForms):
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\e2e\run-e2e.ps1 [-Only notepad,terminal,...]

  WARNING: takes the keyboard focus for ~2 minutes. Do not type while it runs.
#>
param([string[]]$Only = @(), [switch]$KeepData, [switch]$Shots)
$ErrorActionPreference = 'Stop'
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$exe = Join-Path $root 'target\e2e\release\WhistleType.exe'
$cases = Join-Path $root 'tests\audio\generated\cases'
$work = Join-Path $env:TEMP ("wt-e2e-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0') | Out-Null
Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
$wavPointer = Join-Path $work 'wav.txt'
$log = Join-Path $data 'logs\whistletype.log'
$results = New-Object System.Collections.ArrayList
$script:shot = 0

Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Text; using System.Threading; using System.Runtime.InteropServices;
public static class K {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=40)] struct IN { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int size);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  public struct RECT { public int L, T, R, B; }
  public static void Capture(IntPtr h, string path) {
    RECT r; DwmGetWindowAttribute(h, 9, out r, 16);
    using (var b = new System.Drawing.Bitmap(r.R - r.L, r.B - r.T)) {
      using (var g = System.Drawing.Graphics.FromImage(b)) g.CopyFromScreen(r.L, r.T, 0, 0, b.Size);
      b.Save(path, System.Drawing.Imaging.ImageFormat.Png);
    }
  }
  public static void Key(ushort vk, bool up) {
    var i = new IN[1]; i[0].type = 1; i[0].ki.vk = vk; i[0].ki.flags = up ? 2u : 0u;
    SendInput(1, i, Marshal.SizeOf(typeof(IN)));
  }
  public static void Tap(ushort vk) { Key(vk, false); Thread.Sleep(20); Key(vk, true); }
  public static void Chord(ushort mod, ushort vk) { Key(mod, false); Tap(vk); Key(mod, true); }
  public static bool Activate(IntPtr h) {
    for (int i = 0; i < 20; i++) {
      // any input from this process unlocks SetForegroundWindow; 0xE8 is unassigned (an Alt tap would open menus)
      keybd_event(0xE8, 0, 0, UIntPtr.Zero); keybd_event(0xE8, 0, 2, UIntPtr.Zero);
      ShowWindow(h, 9); SetForegroundWindow(h);
      Thread.Sleep(150);
      if (GetForegroundWindow() == h) return true;
    }
    return false;
  }
  public static IntPtr FindTitle(string part) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { if (!IsWindowVisible(h)) return true; var s = new StringBuilder(512); GetWindowText(h, s, 512);
      if (s.ToString().Contains(part)) { found = h; return false; } return true; }, IntPtr.Zero);
    return found;
  }
  public static string Title(IntPtr h) { var s = new StringBuilder(1024); GetWindowText(h, s, 1024); return s.ToString(); }
}
"@

function Find-Window([string]$part, [int]$timeoutMs = 15000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $h = [K]::FindTitle($part)
        if ($h -ne [IntPtr]::Zero) { Start-Sleep -Milliseconds 500; return $h }
        Start-Sleep -Milliseconds 200
    }
    throw "window '$part' did not appear"
}

function Want($name) { return ($Only.Count -eq 0) -or ($Only -contains $name) }
function Set-Wav($name) { Set-Content -Path $wavPointer -Value (Join-Path $cases "$name.wav") -Encoding ASCII }
function Wav-Seconds($name) { $f = Get-Item (Join-Path $cases "$name.wav"); return ($f.Length - 44) / 32000.0 }
function Log-Lines() { if (Test-Path $log) { return @(Get-Content $log -Encoding UTF8) } else { return @() } }

function Wait-Log([int]$from, [string]$pattern, [int]$timeoutMs = 15000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $lines = Log-Lines
        if ($lines.Count -gt $from) {
            $hit = $lines[$from..($lines.Count - 1)] | Where-Object { $_ -match $pattern } | Select-Object -First 1
            if ($hit) { return $hit }
        }
        Start-Sleep -Milliseconds 100
    }
    return $null
}

$sentinel = "CLIPBOARD-SENTINEL-" + (Get-Random)
function Set-Sentinel() { [System.Windows.Forms.Clipboard]::SetText($sentinel) }
function Clipboard-Ok() { try { return [System.Windows.Forms.Clipboard]::GetText() -eq $sentinel } catch { return $false } }

# Holds F8 for the clip length (+ margin) like a user would, then waits for the insert.
function Dictate-Once([IntPtr]$target, [string]$wav, [string]$expectPattern = 'inserted (Pasted|Typed)', [int]$holdExtraMs = 300) {
    Set-Wav $wav
    if (-not [K]::Activate($target)) { throw "could not focus the target window" }
    $from = (Log-Lines).Count
    $hold = [int]((Wav-Seconds $wav) * 1000) + $holdExtraMs
    [K]::Key(0x77, $false)
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $hold) {
        $fg = [K]::GetForegroundWindow()
        if ($fg -ne $target) {
            [K]::Key(0x77, $true)
            $fgPid = 0; [K]::GetWindowThreadProcessId($fg, [ref]$fgPid) | Out-Null
            $who = (Get-Process -Id $fgPid -ErrorAction SilentlyContinue).ProcessName
            throw ("focus moved away during the test (to '{0}' / {1}) - aborted" -f [K]::Title($fg), $who)
        }
        Start-Sleep -Milliseconds 50
    }
    [K]::Key(0x77, $true)
    $released = Get-Date
    $line = Wait-Log $from $expectPattern 60000
    if ($Shots) {
        $script:shot++
        $dir = Join-Path (Join-Path $root 'tests') 'results\shots'; New-Item -ItemType Directory -Force $dir | Out-Null
        [K]::Capture($target, (Join-Path $dir ("{0:D2}-{1}.png" -f $script:shot, $wav)))
    }
    $latency = if ($line -match 'release->inserted (\d+) ms') { [int]$Matches[1] } else { $null }
    return [pscustomobject]@{ line = $line; latency = $latency; from = $from }
}

function Dictate([IntPtr]$target, [string]$wav, [string]$expectPattern = 'inserted (Pasted|Typed)', [int]$holdExtraMs = 300) {
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try { return Dictate-Once $target $wav $expectPattern $holdExtraMs }
        catch {
            Write-Host "  retry ($attempt): $($_.Exception.Message)"
            Start-Sleep 2   # let the cancelled/partial dictation finish
        }
    }
    throw "focus could not be kept on the target window"
}

# Saves and closes the active Notepad tab (Win11 Notepad keeps one process for all tabs; never kill it).
function Close-Tab([IntPtr]$h) {
    if ([K]::Activate($h)) { [K]::Chord(0x11, 0x53); Start-Sleep -Milliseconds 500; [K]::Chord(0x11, 0x57) }
}

function Record($name, $ok, $detail) {
    $null = $results.Add([pscustomobject]@{ test = $name; ok = [bool]$ok; detail = $detail })
    $mark = if ($ok) { 'PASS' } else { 'FAIL' }
    Write-Host ("[{0}] {1}: {2}" -f $mark, $name, $detail)
}

function Has-Words([string]$text, [string[]]$words) {
    $t = $text.ToLower()
    foreach ($w in $words) { if (-not $t.Contains($w.ToLower())) { return $false } }
    return $true
}

# ---------------------------------------------------------------------------------------------------------
$env:WHISTLETYPE_DATA_DIR = $data
$env:WHISTLETYPE_TEST_WAV = $wavPointer
Set-Wav 'sent__pl_basic__paulina_normal'
if ($env:WT_E2E_METHOD) { Set-Content (Join-Path $data 'settings.json') ('{"insert_method":"' + $env:WT_E2E_METHOD + '"}') -Encoding ASCII }
$app = Start-Process $exe -ArgumentList '--background' -PassThru
$netLog = Join-Path $work 'net.txt'
$watch = if ($env:WT_E2E_NO_NETWATCH) { $null } else { Start-Job -ArgumentList $exe, $netLog -ScriptBlock {
    param($exePath, $out)
    while (-not (Test-Path "$out.stop")) {
        foreach ($p in @(Get-Process -Name WhistleType -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exePath })) {
            $tcp = @(Get-NetTCPConnection -OwningProcess $p.Id -ErrorAction SilentlyContinue)
            $udp = @(Get-NetUDPEndpoint -OwningProcess $p.Id -ErrorAction SilentlyContinue)
            foreach ($c in $tcp) { Add-Content $out ("TCP {0}:{1} -> {2}:{3} {4}" -f $c.LocalAddress, $c.LocalPort, $c.RemoteAddress, $c.RemotePort, $c.State) }
            foreach ($c in $udp) { Add-Content $out ("UDP {0}:{1}" -f $c.LocalAddress, $c.LocalPort) }
        }
        Start-Sleep -Milliseconds 250
    }
} }
if (-not (Wait-Log 0 'model ready' 20000)) { throw "app did not load the model" }
[K]::SetProcessDPIAware() | Out-Null
Write-Host "app running (pid $($app.Id)), data $data"

try {
    # --- Notepad -------------------------------------------------------------------------------------
    if (Want 'notepad') {
        $file = Join-Path $work 'notepad.txt'
        Set-Content $file '' -NoNewline
        $np = Start-Process notepad.exe -ArgumentList "`"$file`"" -PassThru
        Start-Sleep 2
        $h = Find-Window 'notepad.txt'
        [K]::Activate($h) | Out-Null; Start-Sleep 1; [K]::Tap(0x23)
        Set-Sentinel
        $r = Dictate $h 'sent__pl_mixed__paulina_normal'
        Start-Sleep -Milliseconds 300
        [K]::Chord(0x11, 0x53)  # Ctrl+S
        Start-Sleep 1
        $text = Get-Content $file -Raw -Encoding UTF8
        Record 'notepad: polish + english terms' ($r.line -and (Has-Words $text @('sprawdź', 'popraw'))) ("'" + $text + "' latency " + $r.latency + " ms")
        Record 'notepad: clipboard restored' (Clipboard-Ok) ([System.Windows.Forms.Clipboard]::GetText())

        # Polish diacritics
        $before = (Get-Content $file -Raw -Encoding UTF8)
        $r = Dictate $h 'sent__pl_story__paulina_normal'
        [K]::Chord(0x11, 0x53); Start-Sleep 1
        $text = (Get-Content $file -Raw -Encoding UTF8).Substring($before.Length)
        Record 'notepad: long sentence with ąęółśżźćń' ($r.line -and (Has-Words $text @('wczoraj', 'pracowałem', 'wersją', 'błędy', 'końcu', 'przygotowałem', 'pamięci'))) ("latency " + $r.latency + " ms, " + $text.Length + " chars")

        # silence / noise must insert nothing
        foreach ($ns in @('ns_digital_silence', 'ns_keyboard_typing', 'ns_breath', 'ns_white_noise')) {
            $before = (Get-Content $file -Raw -Encoding UTF8)
            $r = Dictate $h $ns '(-> (Silence|NoSpeech|TooShort))|inserted'
            [K]::Chord(0x11, 0x53); Start-Sleep -Milliseconds 700
            $after = (Get-Content $file -Raw -Encoding UTF8)
            Record "notepad: $ns inserts nothing" (($after -eq $before) -and ($r.line -notmatch 'inserted')) $r.line
        }

        # rapid hotkey hammering: 15 taps of 20-60 ms
        $before = (Get-Content $file -Raw -Encoding UTF8)
        $from = (Log-Lines).Count
        Set-Wav 'sent__pl_basic__paulina_normal'
        [K]::Activate($h) | Out-Null
        for ($i = 0; $i -lt 15; $i++) { [K]::Key(0x77, $false); Start-Sleep -Milliseconds (20 + 3 * $i); [K]::Key(0x77, $true); Start-Sleep -Milliseconds 25 }
        Start-Sleep 2
        [K]::Chord(0x11, 0x53); Start-Sleep -Milliseconds 700
        $after = (Get-Content $file -Raw -Encoding UTF8)
        $lines = Log-Lines
        $tooShort = @($lines[$from..($lines.Count - 1)] | Where-Object { $_ -match 'TooShort' }).Count
        Record 'notepad: 15 rapid taps -> nothing typed, app alive' (($after -eq $before) -and -not $app.HasExited) "$tooShort recordings rejected as too short"

        # Esc cancels a dictation
        $before = (Get-Content $file -Raw -Encoding UTF8)
        Set-Wav 'sent__pl_tech__paulina_normal'
        [K]::Activate($h) | Out-Null
        $from = (Log-Lines).Count
        [K]::Key(0x77, $false); Start-Sleep 1; [K]::Tap(0x1B); Start-Sleep -Milliseconds 300; [K]::Key(0x77, $true)
        Start-Sleep 2
        [K]::Chord(0x11, 0x53); Start-Sleep -Milliseconds 700
        $after = (Get-Content $file -Raw -Encoding UTF8)
        Record 'notepad: Esc cancels' (($after -eq $before) -and (Wait-Log $from 'cancelled' 1000)) ''

        Close-Tab $h
    }

    # --- 45 s recording (two Whistle segments) -----------------------------------------------------------
    if (Want 'long') {
        $file = Join-Path $work 'long.txt'
        Set-Content $file '' -NoNewline
        $np = Start-Process notepad.exe -ArgumentList "`"$file`"" -PassThru
        Start-Sleep 2
        $h = Find-Window 'long.txt'
        [K]::Activate($h) | Out-Null; Start-Sleep 1; [K]::Tap(0x23)
        $r = Dictate $h 'len_45s'
        [K]::Chord(0x11, 0x53); Start-Sleep 1
        $text = Get-Content $file -Raw -Encoding UTF8
        $segLine = Wait-Log $r.from 'segment\(s\)' 1000
        Record 'long: 45 s dictation split for the 30 s limit' ($r.line -and $segLine -match '2 segment' -and $text.Length -gt 200) ("$($text.Length) chars; $segLine")
        Close-Tab $h
    }

    # --- Terminal (console host / Windows Terminal) -----------------------------------------------------
    if (Want 'terminal') {
        $out = Join-Path $work 'terminal.txt'
        $cmd = "`$host.UI.RawUI.WindowTitle='WT-E2E-TERMINAL'; `$x = Read-Host 'dictate'; Set-Content -Encoding UTF8 -Path '$out' -Value `$x"
        $t = Start-Process powershell.exe -ArgumentList '-NoProfile', '-Command', $cmd -PassThru
        Start-Sleep 3
        $h = Find-Window 'WT-E2E-TERMINAL'
        Set-Sentinel
        $r = Dictate $h 'sent__pl_gradle__paulina_normal'
        Start-Sleep -Milliseconds 400
        [K]::Tap(0x0D)
        Start-Sleep 1.5
        $text = if (Test-Path $out) { Get-Content $out -Raw -Encoding UTF8 } else { '' }
        Record 'terminal: paste into PowerShell prompt' ($r.line -and (Has-Words $text @('gradle', 'sprawdź'))) ("'" + $text.Trim() + "'")
        Record 'terminal: clipboard restored' (Clipboard-Ok) ''
        Stop-Process -Id $t.Id -ErrorAction SilentlyContinue
    }

    # --- Browser (Edge, throw-away profile) ---------------------------------------------------------------
    if (Want 'browser') {
        $edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
        $profile = Join-Path $work 'edge-profile'
        $html = Join-Path $work 'page.html'
        Set-Content $html '<!doctype html><meta charset="utf-8"><title>WTE2E-</title><textarea id=t autofocus style="width:90%;height:200px" oninput="document.title=''WTE2E-''+this.value"></textarea><script>document.getElementById("t").focus()</script>' -Encoding UTF8
        $e = Start-Process $edge -ArgumentList "--user-data-dir=`"$profile`"", '--no-first-run', '--no-default-browser-check', '--new-window', "file:///$($html -replace '\\','/')" -PassThru
        Start-Sleep 5
        $h = Find-Window 'WTE2E-'
        Set-Sentinel
        $r = Dictate $h 'sent__pl_claude__paulina_normal'
        Start-Sleep 1
        $title = [K]::Title($h)
        Record 'browser: Edge textarea' ($r.line -and $title.Length -gt 20) ("'" + $title + "'")
        Record 'browser: clipboard restored' (Clipboard-Ok) ''
        Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object { $_.CommandLine -like "*$profile*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction SilentlyContinue }
    }

    # --- VS Code (new window on a temp file) ----------------------------------------------------------------
    if (Want 'vscode') {
        $file = Join-Path $work 'vscode.txt'
        Set-Content $file '' -NoNewline
        Start-Process code -ArgumentList '-n', "`"$file`"" -WindowStyle Hidden
        Start-Sleep 6
        $h = Find-Window 'vscode.txt'
        if ($false) { }
        else {
            Set-Sentinel
            $r = Dictate $h 'sent__pl_web__paulina_normal'
            Start-Sleep -Milliseconds 500
            [K]::Chord(0x11, 0x53); Start-Sleep 1.5
            $text = Get-Content $file -Raw -Encoding UTF8
            Record 'vscode: editor' ($r.line -and (Has-Words $text @('projekt', 'webgl', 'blender'))) ("'" + $text + "'")
            Record 'vscode: clipboard restored' (Clipboard-Ok) ''
            [K]::Activate($h) | Out-Null
            [K]::Key(0x11, $false); [K]::Key(0x10, $false); [K]::Tap(0x57); [K]::Key(0x10, $true); [K]::Key(0x11, $true) # Ctrl+Shift+W closes the window
        }
    }

    # --- Classic desktop text field (WinForms TextBox) + "type characters" method ------------------------------
    if (Want 'desktop') {
        $out = Join-Path $work 'form.txt'
        $formScript = Join-Path $work 'form.ps1'
        Set-Content $formScript @"
Add-Type -AssemblyName System.Windows.Forms
`$f = New-Object Windows.Forms.Form; `$f.Text = 'WT-E2E-FORM'; `$f.Width = 600; `$f.Height = 200
`$tb = New-Object Windows.Forms.TextBox; `$tb.Multiline = `$true; `$tb.Dock = 'Fill'; `$f.Controls.Add(`$tb)
`$tm = New-Object Windows.Forms.Timer; `$tm.Interval = 200; `$tm.add_Tick({ [IO.File]::WriteAllText('$out', `$tb.Text, [Text.Encoding]::UTF8) }); `$tm.Start()
`$f.add_Shown({ `$tb.Focus() }); [Windows.Forms.Application]::Run(`$f)
"@ -Encoding UTF8
        $fp = Start-Process powershell.exe -ArgumentList '-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass', '-File', $formScript -PassThru
        Start-Sleep 3
        $h = Find-Window 'WT-E2E-FORM'
        Set-Sentinel
        $r = Dictate $h 'sent__pl_tech__adam_normal'
        $text = ''
        for ($i = 0; $i -lt 15 -and -not $text; $i++) { Start-Sleep -Milliseconds 200; if (Test-Path $out) { $text = Get-Content $out -Raw -Encoding UTF8 } }
        Record 'desktop: WinForms TextBox' ($r.line -and (Has-Words $text @('kotlin'))) ("'" + $text + "'")
        Record 'desktop: clipboard restored' (Clipboard-Ok) ''
        Stop-Process -Id $fp.Id -ErrorAction SilentlyContinue
    }

    # --- Toggle mode (restarts the app with other settings, so it runs last) ------------------------------------
    if (Want 'toggle') {
        Stop-Process -Id $app.Id -Force; Start-Sleep 1
        $settingsFile = Join-Path $data 'settings.json'
        $st = Get-Content $settingsFile -Raw | ConvertFrom-Json
        $st.mode = 'toggle'
        $st | ConvertTo-Json -Depth 5 | Set-Content $settingsFile -Encoding UTF8
        $app = Start-Process $exe -ArgumentList '--background' -PassThru
        if (-not (Wait-Log (Log-Lines).Count 'model ready' 20000)) { throw "app did not restart" }
        $file = Join-Path $work 'toggle.txt'
        Set-Content $file '' -NoNewline
        Start-Process notepad.exe -ArgumentList "`"$file`""
        $h = Find-Window 'toggle.txt'
        [K]::Activate($h) | Out-Null; Start-Sleep 1; [K]::Tap(0x23)
        Set-Sentinel
        Set-Wav 'sent__pl_story__adam_normal'
        $from = (Log-Lines).Count
        [K]::Tap(0x77)                                   # press once: start
        Start-Sleep -Milliseconds ([int]((Wav-Seconds 'sent__pl_story__adam_normal') * 1000) + 300)
        [K]::Tap(0x77)                                   # press again: stop
        $line = Wait-Log $from 'inserted (Pasted|Typed)' 20000
        Start-Sleep -Milliseconds 1500
        if ($Shots) { $dir = Join-Path (Join-Path $root 'tests') 'results\shots'; New-Item -ItemType Directory -Force $dir | Out-Null; [K]::Capture($h, (Join-Path $dir 'toggle.png')) }
        [K]::Chord(0x11, 0x53); Start-Sleep 1
        $text = Get-Content $file -Raw -Encoding UTF8
        Record 'toggle mode (press to start, press to stop)' (($line -match 'Pasted') -and (Has-Words $text @('wczoraj', 'pracowałem', 'aplikacji', 'interfejsie', 'notatki', 'pamięci', 'laptopie'))) ("'" + $text + "'")
        Record 'toggle: clipboard restored' (Clipboard-Ok) ''
        Close-Tab $h
    }
}
finally {
    $alive = -not $app.HasExited
    Start-Sleep -Milliseconds 400
    Stop-Process -Id $app.Id -ErrorAction SilentlyContinue
    Record 'app survived all tests' $alive ''
    $errors = @(Log-Lines | Where-Object { $_ -match '\[ERROR\]|panic' })
    Record 'no errors/panics in the log' ($errors.Count -eq 0) ($errors -join ' | ')
    $privacy = @(Log-Lines | Where-Object { $_ -match 'komponent|Gradle|Kotlin|Wczoraj' })
    Record 'log contains no transcript text' ($privacy.Count -eq 0) ''
    Set-Content "$netLog.stop" ''; Start-Sleep -Milliseconds 500; if ($watch) { Stop-Job $watch -ErrorAction SilentlyContinue; Remove-Job $watch -Force -ErrorAction SilentlyContinue }
    $net = if (Test-Path $netLog) { @(Get-Content $netLog | Sort-Object -Unique) } else { @() }
    Record 'no network sockets opened by the app (sampled every 250 ms)' ($net.Count -eq 0) ($net -join '; ')
    $resDir = Join-Path $root 'tests\results'
    New-Item -ItemType Directory -Force $resDir | Out-Null
    $results | ConvertTo-Json | Set-Content (Join-Path $resDir 'e2e.json') -Encoding UTF8
    Copy-Item $log (Join-Path $resDir 'e2e-app.log') -ErrorAction SilentlyContinue
    $pass = @($results | Where-Object ok).Count
    Write-Host "`n$pass / $($results.Count) passed. Log: $resDir\e2e-app.log"
    if (-not $KeepData) { Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue }
}
