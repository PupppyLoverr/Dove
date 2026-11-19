<#
.SYNOPSIS
  One idle sample of Keplorayx in the standardised measurement state, so two
  numbers from two builds and two days can be compared.

  The state, defined once (K22, M2e):
    - one instance only (every keplorayx/keplorayx-engine killed first, and
      the count printed beside the number so a second app is visible)
    - the main window visible at a fixed rect, restored and unoccluded
      (checked against the z-order of every other visible top-level window).
      **Unfocused is the standard state**, not a failure: Windows refuses
      `SetForegroundWindow` from a non-interactive session, so the focused leg
      cannot be relied on. `-NoFocus` skips even the attempt; without it the
      harness tries `AttachThreadInput` and prints whether it got the
      foreground, which is the one comparison worth recording.
    - its own KX_APP_ID, port and copied data dir; never the owner's instance
    - the second number is the same run with the window minimised

  Reports app and engine CPU as % of one core, frames drawn in the sample
  (and how much of the window they damaged), private MB and thread count.

.EXAMPLE
  .\kx-idle-std.ps1 -Tag before -State both -Repeat 5
#>
param(
  [string]$Tag = 'sample',
  [ValidateSet('visible', 'minimised', 'both')][string]$State = 'both',
  [string]$Exe = 'C:\dvw\target-kx-zgx\release\keplorayx.exe',
  [int]$Settle = 35,
  [int]$Seconds = 30,
  [string]$Root = "$env:USERPROFILE\.capy\work\kx-m2e",
  [int]$Port = 27901,
  [hashtable]$Set = @{},
  [int[]]$Rect = @(120, 120, 1240, 820),
  # Where `ui.persist` keeps its state; empty for a first-run state.
  [string]$AppData = '',
  # Extra environment for the launch, `K=V,K=V` (this app's own ablation flags).
  [string]$Flags = '',
  # Skip the foreground attempt entirely (the standard state).
  [switch]$NoFocus,
  [switch]$Json
)
$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class KxWin {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();

  /// Take the foreground for `h` from a non-interactive session: Windows
  /// refuses `SetForegroundWindow` otherwise, but honours it for a thread
  /// attached to the one that holds the foreground.
  public static bool Focus(IntPtr h) {
    IntPtr fg = GetForegroundWindow();
    uint mine = GetCurrentThreadId();
    uint theirs = GetWindowThreadProcessId(fg, IntPtr.Zero);
    bool attached = AttachThreadInput(mine, theirs, true);
    ShowWindow(h, 9);
    BringWindowToTop(h);
    SetForegroundWindow(h);
    SetFocus(h);
    if (attached) AttachThreadInput(mine, theirs, false);
    return GetForegroundWindow() == h;
  }
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int index);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int value, int size);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);

  public static readonly IntPtr HWND_TOP = IntPtr.Zero;
  public const uint SWP_SHOWWINDOW = 0x40, SWP_NOACTIVATE = 0;

  /// Visible top-level windows with a title, top of the z-order first.
  public static List<IntPtr> Visible() {
    var list = new List<IntPtr>();
    EnumWindows((h, p) => {
      if (IsWindowVisible(h) && GetWindowTextLength(h) > 0) list.Add(h);
      return true;
    }, IntPtr.Zero);
    return list;
  }

  /// Every visible titled window of another process that overlaps `r` and
  /// sits above `self` in the z-order (occluding it). The app's own windows
  /// (its launcher, its island) are not occluders.
  public static List<string> Overlapping(IntPtr self, RECT r) {
    var found = new List<string>();
    uint own; GetWindowThreadProcessId(self, out own);
    bool above = true;
    foreach (var h in Visible()) {
      RECT o; if (!GetWindowRect(h, out o)) continue;
      uint pid; GetWindowThreadProcessId(h, out pid);
      bool hits = o.L < r.R && o.R > r.L && o.T < r.B && o.B > r.T;
      if (h == self) { above = false; continue; }
      if (pid == own) continue;
      if (hits && above) found.Add(string.Format("0x{0:x} {1},{2} {3}x{4}", h.ToInt64(), o.L, o.T, o.R - o.L, o.B - o.T));
    }
    return found;
  }
}
'@

function Cores([double]$ms, [int]$sec) { $ms / ($sec * 10) }

# Engine CPU in ms over every engine process (one per instance).
function Engine-Ms {
  $ms = 0.0
  foreach ($g in @(Get-Process keplorayx-engine -ErrorAction SilentlyContinue)) { $ms += $g.TotalProcessorTime.TotalMilliseconds }
  return $ms
}

function Kill-All {
  Get-Process keplorayx, keplorayx-engine -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 1
  $n = @(Get-Process keplorayx, keplorayx-engine -ErrorAction SilentlyContinue).Count
  if ($n -ne 0) { throw "instances still running after kill: $n" }
}

# --- one launch, sampled in one or two states ---------------------------------
function Sample-Run([string]$label) {
  Kill-All
  $data = Join-Path $Root 'data\keplorayx'
  $env:ZERON_ZGX_ENGINE_DATA = $data
  $env:ZERON_ZGX_ENGINE_PORT = "$Port"
  $env:APPDATA = if ($AppData) { $AppData } else { Join-Path $Root 'appdata' }
  $env:KX_APP_ID = 'com.keplorayx.app.m2e'
  $env:ZGX_GPU_STATS = '1'
  Remove-Item env:ZGX_PROFILE -ErrorAction SilentlyContinue
  foreach ($k in $Set.Keys) { Set-Item -Path "env:$k" -Value $Set[$k] }
  foreach ($pair in $Flags.Split(',', [StringSplitOptions]::RemoveEmptyEntries)) {
    $kv = $pair.Split('=', 2)
    Set-Item -Path "env:$($kv[0].Trim())" -Value ($(if ($kv.Count -gt 1) { $kv[1].Trim() } else { '1' }))
  }

  $dir = Join-Path $Root 'logs'
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $log = Join-Path $dir "$label"
  $p = Start-Process -FilePath $Exe -ArgumentList 'main' -WorkingDirectory (Split-Path -Parent $Exe) `
    -RedirectStandardError "$log.err.txt" -RedirectStandardOutput "$log.out.txt" -PassThru
  Start-Sleep -Seconds $Settle
  if ($p.HasExited) { throw "$label`: exited early, code $($p.ExitCode)" }

  $p.Refresh()
  $h = $p.MainWindowHandle
  if ($h -eq [IntPtr]::Zero) { throw "$label`: no main window" }

  # The state: fixed rect, restored, top, focused.
  [KxWin]::ShowWindow($h, 9) | Out-Null                                  # SW_RESTORE
  [KxWin]::SetWindowPos($h, [KxWin]::HWND_TOP, $Rect[0], $Rect[1], $Rect[2], $Rect[3], 0x40) | Out-Null
  if (-not $NoFocus) { [KxWin]::Focus($h) | Out-Null }
  Start-Sleep -Seconds 2
  $r = New-Object KxWin+RECT
  [KxWin]::GetWindowRect($h, [ref]$r) | Out-Null
  $cloaked = 0
  [KxWin]::DwmGetWindowAttribute($h, 14, [ref]$cloaked, 4) | Out-Null     # DWMWA_CLOAKED
  $fore = ([KxWin]::GetForegroundWindow() -eq $h)
  $over = @([KxWin]::Overlapping($h, $r))

  $out = @()
  $states = if ($State -eq 'both') { @('visible', 'minimised') } else { @($State) }
  foreach ($s in $states) {
    if ($s -eq 'minimised') {
      [KxWin]::ShowWindow($h, 6) | Out-Null                                # SW_MINIMIZE
      Start-Sleep -Seconds 4
    }
    $t0 = $p.TotalProcessorTime.TotalMilliseconds
    $e0 = Engine-Ms
    $f0 = 0; $d0 = 0.0
    Select-String -Path "$log.err.txt" -Pattern 'GPU_STATS frames=(\d+)' -AllMatches -ErrorAction SilentlyContinue |
      ForEach-Object { $_.Matches } | ForEach-Object { $f0 += [int]$_.Groups[1].Value }
    $dm0 = (Select-String -Path "$log.err.txt" -Pattern 'damaged_mpx=([0-9.]+)' -AllMatches -ErrorAction SilentlyContinue |
      ForEach-Object { $_.Matches } | ForEach-Object { [double]$_.Groups[1].Value } | Measure-Object -Sum).Sum
    if (-not $dm0) { $dm0 = 0 }
    Start-Sleep -Seconds $Seconds
    $p.Refresh()
    $t1 = $p.TotalProcessorTime.TotalMilliseconds
    $e1 = Engine-Ms
    $f1 = 0
    Select-String -Path "$log.err.txt" -Pattern 'GPU_STATS frames=(\d+)' -AllMatches -ErrorAction SilentlyContinue |
      ForEach-Object { $_.Matches } | ForEach-Object { $f1 += [int]$_.Groups[1].Value }
    $dm1 = (Select-String -Path "$log.err.txt" -Pattern 'damaged_mpx=([0-9.]+)' -AllMatches -ErrorAction SilentlyContinue |
      ForEach-Object { $_.Matches } | ForEach-Object { [double]$_.Groups[1].Value } | Measure-Object -Sum).Sum
    if (-not $dm1) { $dm1 = 0 }

    $procs = @(Get-Process keplorayx -ErrorAction SilentlyContinue).Count
    $out += [pscustomobject]@{
      Label     = $label
      State     = $s
      AppPct    = [math]::Round((Cores ($t1 - $t0) $Seconds), 2)
      EngPct    = [math]::Round((Cores ($e1 - $e0) $Seconds), 2)
      Frames    = $f1 - $f0
      Fps       = [math]::Round(($f1 - $f0) / $Seconds, 2)
      DamagedMpx = [math]::Round($dm1 - $dm0, 2)
      RssMb     = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
      Threads   = $p.Threads.Count
      Instances = $procs
      Focused   = $fore
      Cloaked   = $cloaked
      Overlaps  = $over.Count
      OverWhat  = ($over -join '; ')
      Rect      = "$($r.R - $r.L)x$($r.B - $r.T)@$($r.L),$($r.T)"
    }
  }
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Get-Process keplorayx-engine -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 600
  return $out
}

$rows = @()
$rows += Sample-Run $Tag

if ($Json) { $rows | ConvertTo-Json -Depth 4 } else {
  foreach ($r in $rows) {
    Write-Output ("{0,-12} {1,-10} app {2,6:N2}%  engine {3,5:N2}%  frames {4,4} ({5,5:N2}/s, {6,5:N1} mpx damaged)  rss {7,6:N1} MB  threads {8,2}  instances {9}  focused {10}  cloaked {11}  occluded-by {12}" -f `
        $r.Label, $r.State, $r.AppPct, $r.EngPct, $r.Frames, $r.Fps, $r.DamagedMpx, $r.RssMb, $r.Threads, $r.Instances, $r.Focused, $r.Cloaked, $r.Overlaps)
    if ($r.Overlaps -gt 0) { Write-Output ("             over: {0}" -f $r.OverWhat) }
  }
}
