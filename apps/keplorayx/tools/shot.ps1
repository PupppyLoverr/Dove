# Screenshot a keplorayx window to a PNG: launch it against a scratch
# DOVE_HOME, wait for it to settle, capture its window (PrintWindow with
# PW_RENDERFULLCONTENT, so the composition and glass are in it), close it.
#
#   powershell -File apps\keplorayx\tools\shot.ps1 -Mode gallery -Page icons -Out shots\icons.png
#   powershell -File apps\keplorayx\tools\shot.ps1 -Mode main -Theme orbit-dawn -Out shots\home-dawn.png
param(
    [string]$Mode = "gallery",
    [string]$Page = "",
    [string]$Theme = "",
    [string]$Fixture = "",
    [string]$Out = "shot.png",
    [int]$Seconds = 6,
    [int]$Width = 1240,
    [int]$Height = 820,
    [string]$Exe = "C:\dvw\target-kx-zgx\debug\keplorayx.exe",
    [string]$DataHome = "$env:TEMP\kx-scratch-home"
)
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public class KxShot {
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr value);
  // Physical pixels: a capture of the whole window at any display scale.
  public static void Aware() { SetProcessDpiAwarenessContext(new IntPtr(-4)); }
  // The process's largest visible window.
  public static IntPtr Find(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) {
        RECT r; GetWindowRect(h, out r);
        long a = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
        if (a > area) { area = a; best = h; }
      }
      return true;
    }, IntPtr.Zero);
    return best;
  }
  public static string Capture(IntPtr h, string path) {
    RECT r; GetWindowRect(h, out r);
    int w = r.Right - r.Left, hh = r.Bottom - r.Top;
    if (w <= 0 || hh <= 0) return "empty window";
    using (var bmp = new Bitmap(w, hh, PixelFormat.Format32bppArgb)) {
      using (var g = Graphics.FromImage(bmp)) {
        IntPtr hdc = g.GetHdc();
        PrintWindow(h, hdc, 2);
        g.ReleaseHdc(hdc);
      }
      bmp.Save(path, ImageFormat.Png);
    }
    return w + " x " + hh;
  }
}
"@
[KxShot]::Aware()
New-Item -ItemType Directory -Force $DataHome | Out-Null
$env:DOVE_HOME = $DataHome
$env:KX_PAGE = $Page
$env:KX_THEME = $Theme
$env:KX_FIXTURE = $Fixture
$env:KX_SIZE = "${Width}x${Height}"
$env:KX_SOLID = "1"
$err = "$env:TEMP\kx-shot.err"
$p = Start-Process -FilePath $Exe -ArgumentList @($Mode) -PassThru -RedirectStandardError $err -RedirectStandardOutput "$env:TEMP\kx-shot.out"
$hwnd = [IntPtr]::Zero
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($hwnd -eq [IntPtr]::Zero -and $sw.ElapsedMilliseconds -lt 30000) { Start-Sleep -Milliseconds 100; $hwnd = [KxShot]::Find([uint32]$p.Id) }
if ($hwnd -eq [IntPtr]::Zero) { "no window"; Get-Content $err -TotalCount 30; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; exit 1 }
Start-Sleep -Seconds $Seconds
$hwnd = [KxShot]::Find([uint32]$p.Id)
$dir = Split-Path -Parent $Out
if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
$full = if ([IO.Path]::IsPathRooted($Out)) { [IO.Path]::GetFullPath($Out) } else { [IO.Path]::GetFullPath((Join-Path (Get-Location) $Out)) }
$size = [KxShot]::Capture($hwnd, $full)
Stop-Process -Id $p.Id -Force
"saved $Out ($size)"
Get-Content $err | Where-Object { $_ -match 'error|warning' } | Select-Object -First 20
