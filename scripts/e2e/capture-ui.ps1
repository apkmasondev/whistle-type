<#
.SYNOPSIS  Captures the overlay (listening / transcribing) and the settings window into docs\ for the README.
           Uses the test-hooks build and a throw-away data folder. Windows PowerShell 5.1.
#>
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$work = Join-Path $env:TEMP ("wt-ui-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0') | Out-Null
Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Text; using System.Threading; using System.Runtime.InteropServices;
public static class U {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=40)] struct IN { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int size);
  public static void Key(ushort vk, bool up) { var i = new IN[1]; i[0].type = 1; i[0].ki.vk = vk; i[0].ki.flags = up ? 2u : 0u; SendInput(1, i, Marshal.SizeOf(typeof(IN))); }
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  public static IntPtr FindTitle(string part) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ if(!IsWindowVisible(h)) return true; var s=new StringBuilder(512); GetWindowText(h,s,512); if(s.ToString().Contains(part)){f=h;return false;} return true;}, IntPtr.Zero); return f; }
  public static bool Activate(IntPtr h) { for (int i=0;i<20;i++){ keybd_event(0xE8,0,0,UIntPtr.Zero); keybd_event(0xE8,0,2,UIntPtr.Zero); SetForegroundWindow(h); Thread.Sleep(150); if (GetForegroundWindow()==h) return true;} return false; }
  public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public static uint Pid;
  public static IntPtr Find(string cls) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); uint pid; GetWindowThreadProcessId(h, out pid); if(c.ToString()==cls && IsWindowVisible(h) && pid==Pid){f=h;return false;} return true;}, IntPtr.Zero); return f; }
  public static void Capture(IntPtr h, string path, int pad) {
    RECT r; if (DwmGetWindowAttribute(h, 9, out r, 16) != 0) GetWindowRect(h, out r);
    int x = r.L - pad, y = r.T - pad, w = r.R - r.L + 2*pad, hh = r.B - r.T + 2*pad;
    using (var b = new System.Drawing.Bitmap(w, hh)) { using (var g = System.Drawing.Graphics.FromImage(b)) g.CopyFromScreen(x, y, 0, 0, b.Size); b.Save(path, System.Drawing.Imaging.ImageFormat.Png); }
  }
}
"@
[U]::SetProcessDPIAware() | Out-Null
$docs = Join-Path $root 'docs'
$pointer = Join-Path $work 'wav.txt'
Set-Content $pointer (Join-Path $root 'tests\audio\generated\cases\len_20s.wav')
$env:WHISTLETYPE_DATA_DIR = $data; $env:WHISTLETYPE_TEST_WAV = $pointer; $p = Start-Process (Join-Path $root 'target\e2e\release\WhistleType.exe') -PassThru
[U]::Pid = [uint32]$p.Id
try {
    Start-Sleep 3
    $s = [U]::Find('WhistleType.Settings')
    # the settings / models windows are captured by capture-windows.ps1 (PrintWindow, no screen content)
    $txt = Join-Path $work 'capture.txt'; Set-Content $txt '' -NoNewline
    Start-Process notepad.exe -ArgumentList "`"$txt`""; Start-Sleep 3
    $np = [U]::FindTitle('capture.txt'); if (-not [U]::Activate($np)) { throw 'cannot focus notepad' }
    [U]::Key(0x77, $false)
    Start-Sleep -Milliseconds 3700
    $o = [U]::Find('WhistleType.Overlay'); [U]::Capture($o, (Join-Path $docs 'overlay-listening.png'), 12)
    Start-Sleep -Milliseconds 6000
    [U]::Key(0x77, $true)
    Start-Sleep -Milliseconds 350
    $o = [U]::Find('WhistleType.Overlay'); if ($o -ne [IntPtr]::Zero) { [U]::Capture($o, (Join-Path $docs 'overlay-transcribing.png'), 12) }
    Start-Sleep 3
    Set-Content $pointer (Join-Path $root 'tests\audio\generated\cases\ns_keyboard_typing.wav')
    [U]::Key(0x77, $false); Start-Sleep -Milliseconds 1500; [U]::Key(0x77, $true); Start-Sleep -Milliseconds 400
    $o = [U]::Find('WhistleType.Overlay'); if ($o -ne [IntPtr]::Zero) { [U]::Capture($o, (Join-Path $docs 'overlay-nospeech.png'), 12) }
    # close our tab (Ctrl+S, Ctrl+W)
    if ([U]::Activate($np)) { [U]::Key(0x11,$false); [U]::Key(0x53,$false); [U]::Key(0x53,$true); Start-Sleep -Milliseconds 400; [U]::Key(0x57,$false); [U]::Key(0x57,$true); [U]::Key(0x11,$true) }
} finally {
    Start-Sleep 1
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
Get-ChildItem $docs *.png | Select-Object Name, Length
