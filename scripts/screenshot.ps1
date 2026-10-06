<#
.SYNOPSIS  Dev helper: captures a visible top-level window (by window class) or the primary screen to a PNG.
#>
param([string]$Class = '', [string]$Out = 'shot.png', [int]$ProcessId = 0)
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class Shot {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(P cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public struct RECT { public int L, T, R, B; }
  public static IntPtr Find(string cls, uint procId) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { var c = new StringBuilder(256); GetClassName(h, c, 256); uint pid; GetWindowThreadProcessId(h, out pid);
      if (c.ToString() == cls && IsWindowVisible(h) && (procId == 0 || pid == procId)) { found = h; return false; } return true; }, IntPtr.Zero);
    return found;
  }
}
"@
[Shot]::SetProcessDPIAware() | Out-Null
$r = New-Object Shot+RECT
if ($Class) {
    $h = [Shot]::Find($Class, [uint32]$ProcessId)
    if ($h -eq [IntPtr]::Zero) { throw "no visible window of class '$Class'" }
    if ([Shot]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) -ne 0) { [Shot]::GetWindowRect($h, [ref]$r) | Out-Null }
} else {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $r.L = $b.Left; $r.T = $b.Top; $r.R = $b.Right; $r.B = $b.Bottom
}
$w = $r.R - $r.L; $hgt = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size $w, $hgt))
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
"$Out ${w}x${hgt}"
