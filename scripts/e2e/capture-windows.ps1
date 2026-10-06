<#
.SYNOPSIS  Captures the Settings and Speech models windows (English and Polish) with PrintWindow - only the app's own
           windows are rendered, never the screen. Test-hooks build, throw-away data folder. Windows PowerShell 5.1.
.EXAMPLE   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\e2e\capture-windows.ps1 -Out docs
#>
param([string]$Out = 'docs')
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$outDir = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $root $Out }
# inside the repository (git-ignored target\), so paths shown in the windows contain no user name
$work = Join-Path $root ('target\capture-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force (Join-Path $data 'models\whistle-2.0.0') | Out-Null
Copy-Item (Join-Path $root 'third_party\needle\whistle.cact') (Join-Path $data 'models\whistle-2.0.0\')
Set-Content (Join-Path $data 'settings.json') '{"ui_language":"en"}' -Encoding ASCII
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class C {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public struct RECT { public int L, T, R, B; }
  public static uint Pid;
  public static IntPtr Find(string cls) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ var c=new StringBuilder(256); GetClassName(h,c,256); uint pid; GetWindowThreadProcessId(h, out pid); if(c.ToString()==cls && IsWindowVisible(h) && pid==Pid){f=h;return false;} return true;}, IntPtr.Zero); return f; }
  public static void Capture(IntPtr h, string path) {
    RECT r; GetWindowRect(h, out r);
    using (var b = new System.Drawing.Bitmap(r.R - r.L, r.B - r.T)) {
      using (var g = System.Drawing.Graphics.FromImage(b)) { var dc = g.GetHdc(); PrintWindow(h, dc, 2); g.ReleaseHdc(dc); }
      b.Save(path, System.Drawing.Imaging.ImageFormat.Png);
    }
  }
}
"@
[C]::SetProcessDPIAware() | Out-Null
function Win($cls) { for ($i = 0; $i -lt 50; $i++) { $h = [C]::Find($cls); if ($h -ne [IntPtr]::Zero) { return $h }; Start-Sleep -Milliseconds 100 }; throw "no window $cls" }
function Click($win, $id) { [C]::SendMessage($win, 0x0111, [IntPtr]$id, [C]::GetDlgItem($win, $id)) | Out-Null; Start-Sleep -Milliseconds 700 }
$env:WHISTLETYPE_DATA_DIR = $data
$p = Start-Process (Join-Path $root 'target\e2e\release\WhistleType.exe') -PassThru
[C]::Pid = [uint32]$p.Id
try {
    foreach ($lang in 'en', 'pl') {
        $s = Win 'WhistleType.Settings'; Start-Sleep 1.5
        [C]::Capture($s, (Join-Path $outDir "settings-$lang.png"))
        Click $s 127                                     # Models…
        $m = Win 'WhistleType.Models'; Start-Sleep 0.5
        [C]::Capture($m, (Join-Path $outDir "models-$lang.png"))
        if ($lang -eq 'en') {
            $combo = [C]::GetDlgItem($s, 124)
            [C]::SendMessage($combo, 0x014E, [IntPtr]0, [IntPtr]::Zero) | Out-Null       # Polski
            [C]::SendMessage($s, 0x0111, [IntPtr]((1 -shl 16) -bor 124), $combo) | Out-Null
            Start-Sleep 2
        }
    }
} finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
Get-ChildItem $outDir -Filter '*-??.png' | Select-Object Name, Length
