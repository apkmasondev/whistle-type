<#
.SYNOPSIS  Settings window smoke test through UI Automation (test-hooks build). Windows PowerShell 5.1.
           Mic test (real microphone, nothing recorded), vocabulary add/remove, shortcut capture, autostart
           registry value, overlay off. Restores the Run key at the end.
#>
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$work = Join-Path $env:TEMP ("wt-ui-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0') | Out-Null
Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
$settingsFile = Join-Path $data 'settings.json'
Set-Content $settingsFile '{"ui_language":"en"}' -Encoding ASCII   # assertions below use the English captions
$log = Join-Path $data 'logs\whistletype.log'
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class S {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=40)] struct IN { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int size);
  public static void Key(ushort vk, bool up) { var i = new IN[1]; i[0].type = 1; i[0].ki.vk = vk; i[0].ki.flags = up ? 2u : 0u; SendInput(1, i, Marshal.SizeOf(typeof(IN))); }
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P cb, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, P cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static IntPtr Dialog() { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); uint pid; GetWindowThreadProcessId(h, out pid); if (c.ToString()=="#32770" && IsWindowVisible(h) && pid==Pid) { f=h; return false; } return true; }, IntPtr.Zero); return f; }
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public static uint Pid;
  public static IntPtr Top(string cls) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); uint pid; GetWindowThreadProcessId(h, out pid); if (c.ToString()==cls && IsWindowVisible(h) && pid==Pid) { f=h; return false; } return true; }, IntPtr.Zero); return f; }
  public static IntPtr Child(IntPtr parent, string cls, string text) { IntPtr f = IntPtr.Zero; EnumChildWindows(parent, (h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); var t=new StringBuilder(512); GetWindowText(h,t,512);
      if (c.ToString()==cls && (text==null || t.ToString()==text)) { f=h; return false; } return true; }, IntPtr.Zero); return f; }
}
"@
$results = @()
function Record($n, $ok, $d) { $script:results += [pscustomobject]@{ test = $n; ok = [bool]$ok; detail = $d }; Write-Host ("[{0}] {1}: {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $n, $d) }
function Win($cls) { for ($i = 0; $i -lt 50; $i++) { $h = [S]::Top($cls); if ($h -ne [IntPtr]::Zero) { return $h }; Start-Sleep -Milliseconds 100 }; throw "no window $cls" }
function Ctl($win, $cls, $text) { $h = [S]::Child($win, $cls, $text); if ($h -eq [IntPtr]::Zero) { throw "no control '$text'" }; $h }
function Invoke($win, $text) { [S]::SendMessage((Ctl $win 'Button' $text), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 300 }  # BM_CLICK
function Toggle($win, $text) { Invoke $win $text }
function SelectCombo($win, $id, $index) {
    $combo = [S]::GetDlgItem($win, $id)
    [S]::SendMessage($combo, 0x014E, [IntPtr]$index, [IntPtr]::Zero) | Out-Null                 # CB_SETCURSEL
    [S]::SendMessage($win, 0x0111, [IntPtr]((1 -shl 16) -bor $id), $combo) | Out-Null            # WM_COMMAND CBN_SELCHANGE
    Start-Sleep -Milliseconds 400
}
function Settings() { Get-Content $settingsFile -Raw | ConvertFrom-Json }
function RunValue() { (Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run -ErrorAction SilentlyContinue).WhistleType }

$runBefore = RunValue
$env:WHISTLETYPE_DATA_DIR = $data; $p = Start-Process (Join-Path $root 'target\e2e\release\WhistleType.exe') -PassThru
[S]::Pid = [uint32]$p.Id
try {
    $w = Win 'WhistleType.Settings'
    Record 'settings window opens' $true ''

    Invoke $w 'Test'; Start-Sleep 1.5
    $txt = (Get-Content $log -Raw)
    Record 'microphone test opens the real microphone' ($txt -match 'audio: capture started on') (([regex]::Match($txt, 'capture started on "[^"]+"')).Value)
    Invoke $w 'Stop'
    Record 'microphone test stops' ((Get-Content $log -Raw) -match 'audio: capture stopped') ''

    Invoke $w 'Manage…'
    $v = Win 'WhistleType.Vocabulary'
    $edit = Ctl $v 'Edit' $null
    [S]::SendMessage($edit, 0x000C, [IntPtr]::Zero, 'Kubernetes') | Out-Null   # WM_SETTEXT
    Invoke $v 'Add'
    Record 'vocabulary: add entry saved' ((Settings).vocabulary -contains 'Kubernetes') ''
    $list = Ctl $v 'ListBox' $null
    $idx = [S]::SendMessage($list, 0x01A2, [IntPtr]::Zero, 'Kubernetes')       # LB_FINDSTRINGEXACT
    [S]::SendMessage($list, 0x0186, $idx, [IntPtr]::Zero) | Out-Null          # LB_SETCURSEL
    Invoke $v 'Remove'
    Record 'vocabulary: remove entry saved' (-not ((Settings).vocabulary -contains 'Kubernetes')) "$(@((Settings).vocabulary).Count) entries"
    Invoke $v 'Close'

    [S]::SetForegroundWindow($w) | Out-Null
    Invoke $w 'F8'
    [S]::Key(0x11, $false); [S]::Key(0x78, $false); [S]::Key(0x78, $true); [S]::Key(0x11, $true); Start-Sleep -Milliseconds 500
    $hk = (Settings).hotkey
    Record 'shortcut capture: Ctrl+F9 saved' (($hk.vk -eq 0x78) -and $hk.ctrl) ($hk | ConvertTo-Json -Compress)
    Invoke $w 'Ctrl+F9'
    [S]::Key(0x77, $false); [S]::Key(0x77, $true); Start-Sleep -Milliseconds 500
    $hk = (Settings).hotkey
    Record 'shortcut capture: back to F8' (($hk.vk -eq 0x77) -and -not $hk.ctrl) ''
    Invoke $w 'F8'
    [S]::Key(0x41, $false); [S]::Key(0x41, $true); Start-Sleep -Milliseconds 500
    Record 'shortcut capture rejects a plain letter' ((Settings).hotkey.vk -eq 0x77) ''

    Toggle $w 'Run WhistleType when I sign in'
    $rv = RunValue
    Record 'start with Windows writes HKCU Run' ($rv -like '*WhistleType.exe" --background') $rv
    Toggle $w 'Run WhistleType when I sign in'
    Record 'start with Windows off removes it' ($null -eq (RunValue)) ''

    # ---- Recognition: engine mode, accurate model, GPU, Model Manager ---------------------------------
    SelectCombo $w 125 1
    Record 'engine mode FAST saved' ((Settings).engine_mode -eq 'fast') ''
    SelectCombo $w 125 2
    Record 'engine mode ACCURATE saved' ((Settings).engine_mode -eq 'accurate') ''
    SelectCombo $w 125 0
    Record 'engine mode AUTO saved' ((Settings).engine_mode -eq 'auto') ''
    $gpuBox = [S]::GetDlgItem($w, 128)
    if ([S]::IsWindowEnabled($gpuBox)) {
        Toggle $w 'Use the NVIDIA GPU (CUDA) for Whisper'
        $off = -not (Settings).use_gpu
        Toggle $w 'Use the NVIDIA GPU (CUDA) for Whisper'
        Record 'GPU checkbox saved' ($off -and (Settings).use_gpu) ''
    }
    # choosing a model that is not downloaded selects it and opens the Model Manager
    SelectCombo $w 126 1
    $m = Win 'WhistleType.Models'
    Record 'accurate model choice saved, Model Manager opens' ((Settings).whisper_model -eq 'whisper-small') (Settings).whisper_model
    $lv = Ctl $m 'SysListView32' $null
    $rows = [int][S]::SendMessage($lv, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero)                    # LVM_GETITEMCOUNT
    Record 'Model Manager lists Whistle, 4 Whisper models and the GPU pack' ($rows -eq 6) "$rows rows"
    $dl = Ctl $m 'Button' 'Download'; $del = Ctl $m 'Button' 'Delete'
    Record 'Model Manager: Download enabled, Delete disabled for a missing model' ([S]::IsWindowEnabled($dl) -and -not [S]::IsWindowEnabled($del)) ''
    # Download asks first; cancelling must not touch the network
    [S]::PostMessage($dl, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null                     # BM_CLICK (async: a modal box follows)
    $dlg = [IntPtr]::Zero; for ($i = 0; $i -lt 30 -and $dlg -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $dlg = [S]::Dialog() }
    Record 'Download asks for confirmation' ($dlg -ne [IntPtr]::Zero) ''
    if ($dlg -ne [IntPtr]::Zero) { [S]::PostMessage($dlg, 0x0111, [IntPtr]2, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 500 }   # IDCANCEL
    Record 'cancelled download makes no request' (-not ((Get-Content $log -Raw) -match 'download: https')) ''
    Invoke $m 'Close'
    SelectCombo $w 126 3
    Invoke $w 'Models…'
    $m = Win 'WhistleType.Models'

    # switch the interface to Polish: windows are rebuilt in the new language
    Invoke $w 'Manage…'
    $combo = [S]::GetDlgItem($w, 124)
    [S]::SendMessage($combo, 0x014E, [IntPtr]0, [IntPtr]::Zero) | Out-Null                       # CB_SETCURSEL 0 = Polski
    [S]::SendMessage($w, 0x0111, [IntPtr]((1 -shl 16) -bor 124), $combo) | Out-Null               # WM_COMMAND CBN_SELCHANGE
    Start-Sleep 1.5
    $w2 = Win 'WhistleType.Settings'
    $pl = ([S]::Child($w2, 'Button', 'Zamknij') -ne [IntPtr]::Zero) -and ([S]::Child($w2, 'Button', 'Zarządzaj…') -ne [IntPtr]::Zero)
    $v2 = Win 'WhistleType.Vocabulary'
    $plv = [S]::Child($v2, 'Button', 'Przywróć domyślne') -ne [IntPtr]::Zero
    Record 'interface language: settings + vocabulary rebuilt in Polish' ($pl -and $plv -and (Settings).ui_language -eq 'pl') ("settings=$pl vocab=$plv saved=$((Settings).ui_language)")
    $m2 = Win 'WhistleType.Models'
    Record 'interface language: Model Manager rebuilt in Polish' ([S]::Child($m2, 'Button', 'Pobierz') -ne [IntPtr]::Zero) ''
    $combo = [S]::GetDlgItem($w2, 124)
    [S]::SendMessage($combo, 0x014E, [IntPtr]1, [IntPtr]::Zero) | Out-Null
    [S]::SendMessage($w2, 0x0111, [IntPtr]((1 -shl 16) -bor 124), $combo) | Out-Null
    Start-Sleep 1.5
    $w3 = Win 'WhistleType.Settings'
    Record 'interface language: back to English' (([S]::Child($w3, 'Button', 'Close') -ne [IntPtr]::Zero) -and (Settings).ui_language -eq 'en') ''
}
finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    if ($runBefore) { Set-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run -Name WhistleType -Value $runBefore }
    $errs = @(Get-Content $log | Where-Object { $_ -match '\[ERROR\]|panic' })
    Record 'no errors in the log' ($errs.Count -eq 0) ($errs -join ' | ')
    Write-Host ("{0} / {1} passed" -f @($results | Where-Object ok).Count, $results.Count)
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
