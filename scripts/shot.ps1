<#
.SYNOPSIS
  A real picture of a running Keplorayx window: PrintWindow with
  PW_RENDERFULLCONTENT, saved as a PNG, with the count of luminance levels that
  says whether it is a picture of something.

.DESCRIPTION
  The window is drawn by zgx's GPU renderer, so the two obvious ways to read it
  back return nothing on Windows:

  - `BitBlt` off `GetWindowDC`, and `PrintWindow` with flag 0 (`PW_CLIENTONLY`),
    write pure black for this window (mean luma 0, one level).
  - `CopyFromScreen` reads the desktop, so it returns whatever is stacked on
    top of the window, not the window.

  Flag 2, `PW_RENDERFULLCONTENT`, asks DWM for the window's own composited
  surface and does return the window: the main window at 1550x830 reads ~210
  distinct grey levels in its dark theme, ~154 in light.

  A capture taken before the first frame is a single flat fill, so the script
  waits (up to -WaitSeconds) until the frame carries at least -MinLevels
  distinct luminance levels. A frame that never drew is reported and no file is
  written.

.PARAMETER Exe
  The app whose process's window to capture. Without it, -Title is matched
  against every visible window.

.PARAMETER Title
  The window title to match (substring). Keplorayx's main window is
  "Keplorayx"; the launcher window is "Keplorayx launcher" and lives offscreen,
  so it is never the one that matches this.

.PARAMETER Out
  Where the PNG goes.

.PARAMETER WaitSeconds
  How long to wait for a frame with content. Default 120: a cold launch on this
  laptop takes 9-53 s to draw its first frame.

.PARAMETER MinLevels
  Distinct luminance levels a frame needs to count as drawn. Default 60 (a
  blank fill has 1; the drawn main window has ~210).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\shot.ps1 -Exe C:\dvw\target-kx-zgx\debug\keplorayx.exe -Out $env:TEMP\main.png

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\shot.ps1 -Out $env:TEMP\main.png
#>
param(
  [string]$Exe,
  [string]$Title = "Keplorayx",
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$WaitSeconds = 120,
  [int]$MinLevels = 60
)

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class KxShot {
  public delegate bool Proc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flag);

  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

  /// Visible top-level windows of `pids` (any process when null) whose title
  /// holds `title` (any title when null or empty) and that are big enough to
  /// be a window rather than a helper surface.
  public static List<IntPtr> Find(uint[] pids, string title) {
    var found = new List<IntPtr>();
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      if (!IsWindowVisible(h)) return true;
      var sb = new StringBuilder(512);
      GetWindowTextW(h, sb, 512);
      var text = sb.ToString();
      if (pids != null) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        if (Array.IndexOf(pids, pid) < 0) return true;
      }
      if (!String.IsNullOrEmpty(title) && text.IndexOf(title, StringComparison.OrdinalIgnoreCase) < 0) return true;
      RECT r; GetWindowRect(h, out r);
      if (r.R - r.L > 200 && r.B - r.T > 200) found.Add(h);
      return true;
    }, IntPtr.Zero);
    return found;
  }

  /// The window's own composited surface, drawn into a .NET bitmap's DC.
  /// PW_RENDERFULLCONTENT (2) is the flag that returns this app's pixels; 0
  /// and 1 return black. Null when it fails.
  public static System.Drawing.Bitmap Grab(IntPtr h) {
    RECT r; GetWindowRect(h, out r);
    int w = r.R - r.L, ht = r.B - r.T;
    if (w <= 0 || ht <= 0) return null;
    var bmp = new System.Drawing.Bitmap(w, ht, System.Drawing.Imaging.PixelFormat.Format32bppArgb);
    using (var g = System.Drawing.Graphics.FromImage(bmp)) {
      IntPtr dc = g.GetHdc();
      bool ok = PrintWindow(h, dc, 2);
      g.ReleaseHdc(dc);
      if (!ok) { bmp.Dispose(); return null; }
    }
    return bmp;
  }
}
"@

$pids = $null
if ($Exe) {
  $name = [System.IO.Path]::GetFileNameWithoutExtension($Exe)
  $found = @(Get-Process -Name $name -ErrorAction SilentlyContinue | ForEach-Object { [uint32]$_.Id })
  if ($found.Count -eq 0) { Write-Error "no $name process is running ($Exe)"; exit 1 }
  $pids = [uint32[]]$found
}

$deadline = (Get-Date).AddSeconds($WaitSeconds)
$done = $false
$lastLevels = 0
while (-not $done -and (Get-Date) -lt $deadline) {
  $windows = [KxShot]::Find($pids, $Title)
  foreach ($h in $windows) {
    if ([KxShot]::IsIconic($h)) { continue }
    $bmp = [KxShot]::Grab($h)
    if ($null -eq $bmp) { continue }
    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    for ($y = 0; $y -lt $bmp.Height; $y += 4) {
      for ($x = 0; $x -lt $bmp.Width; $x += 4) {
        $c = $bmp.GetPixel($x, $y)
        [void]$seen.Add([int](0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B))
      }
    }
    $lastLevels = $seen.Count
    if ($lastLevels -ge $MinLevels) {
      $dir = Split-Path -Parent $Out
      if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
      $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
      "captured {0}x{1} hwnd={2} levels={3} -> {4}" -f $bmp.Width, $bmp.Height, $h, $lastLevels, $Out
      $bmp.Dispose()
      $done = $true
      break
    }
    $bmp.Dispose()
  }
  if (-not $done) { Start-Sleep -Milliseconds 1500 }
}
if (-not $done) {
  Write-Error ("no frame with content in {0}s (last: {1} levels): the window has not drawn, or it is the wrong window" -f $WaitSeconds, $lastLevels)
  exit 1
}
