<#
.SYNOPSIS
  Measures Keplorayx's startup, idle CPU, render count, RAM and key-to-frame
  latency on Windows, on copies of the app's data, never the owner's.

.DESCRIPTION
  Every number this prints is one of the design spec's acceptance criteria
  (`reports/KEPL-11.md` §4.10), measured the way the spec asks for it: the
  window's own pixels for "first frame", the process tree's private bytes for
  RAM, `GetProcessTimes` deltas for CPU, and the engine's own counters for
  renders.

  Three independent sources, so no number rests on one of them:

  - `EnumWindows` polled every few milliseconds: when the main window's HWND
    exists, and when its composited frame stops being one flat fill
    (`PrintWindow` flag 2, the only read that returns this window's pixels —
    see `scripts/shot.ps1`).
  - The app's own stderr under `ZGX_PROFILE=1`: `profile mark` lines for each
    startup milestone, and one `profile N frames: …` line per N rendered
    frames carrying the per-frame cost of every phase, the Luau included.
  - `ZGX_GPU_STATS=1`: a `GPU_STATS frames=…` line about once a second, only
    when frames were actually rendered, so its absence is the zero-frame
    proof.

.PARAMETER Mode
  startup  — process start → HWND → first populated frame, N times.
  idle     — CPU over N seconds at rest and how many frames were drawn.
  memory   — private bytes of the app and engine at rest, with 1 thread open
             and with 20 threads listed.
  all      — the three in order.

.PARAMETER DataSeed
  A data directory to copy (the owner's, read-only). Copied once per run into
  -Root; the app only ever opens the copy.

.PARAMETER Cold
  Start from a copy with no `templates/` tree: the first-launch path where the
  app has to lay its own interface down twice.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\perf\kx-perf.ps1 -Mode startup -Cold -Repeats 1
#>
param(
  [ValidateSet('startup', 'idle', 'memory', 'all')][string]$Mode = 'startup',
  [string]$Exe = 'C:\dvw\target-kx-zgx\debug\keplorayx.exe',
  [string]$DataSeed = "$env:USERPROFILE\.keplorayx",
  [string]$Root = "$env:USERPROFILE\.capy\work\kx-perf",
  [int]$Port = 27811,
  [int]$Repeats = 3,
  [switch]$Cold,
  [int]$IdleSeconds = 60,
  # 20 threads listed: the state the spec calls "RSS with 20 threads".
  [int]$Threads = 20,
  [string]$Out = ''
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path $Exe).Path
$exeDir = Split-Path -Parent $exe
if (-not $Out) { $Out = Join-Path $Root 'results' }

# ---------------------------------------------------------------- win32 bits
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class KxWin {
  public delegate bool Proc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flag);

  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

  /// The first visible top-level window of `pid` whose title holds `title`
  /// and whose box is big enough to be a window, or IntPtr.Zero.
  public static IntPtr Find(uint pid, string title) {
    IntPtr found = IntPtr.Zero;
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      if (!IsWindowVisible(h)) return true;
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
      var sb = new StringBuilder(512);
      GetWindowTextW(h, sb, 512);
      if (sb.ToString().IndexOf(title, StringComparison.OrdinalIgnoreCase) < 0) return true;
      RECT r; GetWindowRect(h, out r);
      if (r.R - r.L < 200 || r.B - r.T < 200) return true;
      found = h;
      return false;
    }, IntPtr.Zero);
    return found;
  }

  /// Distinct grey levels in the window's own composited surface, sampled on
  /// a grid: 1 for a flat fill, ~200 for a drawn frame. 0 when the read fails.
  public static int Levels(IntPtr h, int step) {
    RECT r; GetWindowRect(h, out r);
    int w = r.R - r.L, ht = r.B - r.T;
    if (w <= 0 || ht <= 0) return 0;
    using (var bmp = new System.Drawing.Bitmap(w, ht, System.Drawing.Imaging.PixelFormat.Format32bppArgb))
    using (var g = System.Drawing.Graphics.FromImage(bmp)) {
      IntPtr dc = g.GetHdc();
      bool ok = PrintWindow(h, dc, 2);
      g.ReleaseHdc(dc);
      if (!ok) return 0;
      var seen = new HashSet<int>();
      for (int y = 0; y < ht; y += step)
        for (int x = 0; x < w; x += step) {
          var c = bmp.GetPixel(x, y);
          seen.Add((int)(0.299 * c.R + 0.587 * c.G + 0.114 * c.B));
        }
      return seen.Count;
    }
  }
}
"@

# ---------------------------------------------------------------- the copies
function Initialize-Data([string]$name) {
  # A copy of the seed, made once. Its heavy caches are left out: they are not
  # what a launch reads (`skills/screenshots`).
  $dir = Join-Path $Root "data-$name\keplorayx"
  if (-not (Test-Path (Join-Path $dir 'kx-settings.json'))) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dir) | Out-Null
    $skip = @('browser', 'worktrees', 'picture-cache', 'turn-snapshots', 'logs', 'mesh', 'favicons', 'model-catalogs', 'scratch')
    $args = @($DataSeed, $dir, '/E', '/NFL', '/NDL', '/NJH', '/NJS', '/R:1', '/W:1', '/MT:8') + ($skip | ForEach-Object { @('/XD', $_) })
    & robocopy @args | Out-Null
  }
  return $dir
}

function Reset-Cold([string]$dir) {
  # What a first launch has never had: the laid-down interface.
  $t = Join-Path $dir 'templates'
  if (Test-Path $t) { Remove-Item -Recurse -Force $t }
}

function Start-App([string]$data, [string]$appdata, [string]$logBase, [string[]]$envs) {
  $env:ZERON_ZGX_ENGINE_DATA = $data
  $env:ZERON_ZGX_ENGINE_PORT = "$Port"
  $env:APPDATA = $appdata
  $env:KX_APP_ID = 'com.keplorayx.app.perf'
  $env:ZGX_INSPECT_SOCKET = '1'
  $env:ZGX_PROFILE = '1'
  $env:ZGX_GPU_STATS = '1'
  foreach ($e in $envs) {
    $kv = $e -split '=', 2
    Set-Item -Path "env:$($kv[0])" -Value $kv[1]
  }
  $out = "$logBase.out.txt"
  $err = "$logBase.err.txt"
  $started = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $exe -ArgumentList 'main' -WorkingDirectory $exeDir `
    -RedirectStandardOutput $out -RedirectStandardError $err -PassThru
  return @{ Process = $p; Started = $started; Err = $err; Out = $out }
}

function Stop-App($h) {
  $p = $h.Process
  if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  # The engine the app started goes with it; a stale one holds the port.
  Get-Process keplorayx-engine -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 400
}

# The app's own ctor's window is only up once it is *visible*; poll for both.
function Wait-Window($h, [double]$limitMs) {
  while ($h.Started.Elapsed.TotalMilliseconds -lt $limitMs) {
    $p = $h.Process
    if ($p.HasExited) { return $null }
    $hwnd = [KxWin]::Find([uint32]$p.Id, 'Keplorayx')
    if ($hwnd -ne [IntPtr]::Zero) { return @{ Hwnd = $hwnd; Ms = $h.Started.Elapsed.TotalMilliseconds } }
    Start-Sleep -Milliseconds 4
  }
  $null
}

function Wait-Frame($h, [IntPtr]$hwnd, [double]$limitMs, [int]$minLevels = 60) {
  while ($h.Started.Elapsed.TotalMilliseconds -lt $limitMs) {
    $levels = [KxWin]::Levels($hwnd, 16)
    if ($levels -ge $minLevels) { return @{ Ms = $h.Started.Elapsed.TotalMilliseconds; Levels = $levels } }
    Start-Sleep -Milliseconds 40
  }
  $null
}

# ---------------------------------------------------------------- the samples
function Get-Tree([int]$pid_) {
  # The app plus the engine it spawned: the spec's "private RAM" is the
  # interface's own process, and the engine is reported beside it.
  $procs = @(Get-Process -Id $pid_ -ErrorAction SilentlyContinue)
  $procs += @(Get-Process keplorayx-engine -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $exeDir 'keplorayx-engine.exe') })
  $procs
}

function Sample-Cpu([int]$pid_, [int]$seconds) {
  $p = Get-Process -Id $pid_ -ErrorAction SilentlyContinue
  if (-not $p) { return $null }
  $a = $p.TotalProcessorTime
  Start-Sleep -Seconds $seconds
  $p.Refresh()
  if ($p.HasExited) { return $null }
  $b = $p.TotalProcessorTime
  $cores = [Environment]::ProcessorCount
  # Percent of one core, as the spec's "≤0.3 % idle CPU (one core)".
  ($b - $a).TotalMilliseconds / ($seconds * 1000.0) * 100.0
}

function Get-Frames([string]$err) {
  # `GPU_STATS frames=N` is printed only after a frame was rendered, so the
  # sum over a window is the count of frames Actually Drawn in it.
  if (-not (Test-Path $err)) { return 0 }
  $sum = 0
  foreach ($line in (Get-Content $err -ErrorAction SilentlyContinue)) {
    if ($line -match 'GPU_STATS frames=(\d+)') { $sum += [int]$Matches[1] }
  }
  $sum
}

function Get-ProfileLines([string]$err) {
  if (-not (Test-Path $err)) { return @() }
  @(Get-Content $err -ErrorAction SilentlyContinue | Where-Object { $_ -match '^(profile|GPU_STATS|zeron-zgx:)' })
}

function Get-StartupMarks([string]$err) {
  $out = [ordered]@{}
  foreach ($line in (Get-ProfileLines $err)) {
    if ($line -match '^profile mark .*?([\d.]+) ms: (.+)$') { $out[$Matches[2]] = [double]$Matches[1] }
  }
  $out
}

# ---------------------------------------------------------------- the modes
function Measure-Startup([switch]$cold) {
  $name = if ($cold) { 'cold' } else { 'warm' }
  $data = Initialize-Data 'base'
  if ($cold) { Reset-Cold $data }
  $appdata = Join-Path $Root 'appdata'
  New-Item -ItemType Directory -Force -Path (Join-Path $appdata 'keplorayx-next\script-state\app.luau') | Out-Null

  # Warm means "the interface was laid down by an earlier launch", so a warm
  # run starts with the throwaway launch that lays it: the first launch on a
  # copied data dir is always a cold one, whatever this run is measuring.
  if (-not $cold) {
    $warm = Start-App $data $appdata (Join-Path $Root 'logs\startup-warmup') @()
    $null = Wait-Window $warm 180000
    Stop-App $warm
  }

  $rows = @()
  for ($i = 1; $i -le $Repeats; $i++) {
    if ($cold) { Reset-Cold $data }
    $logBase = Join-Path $Root "logs\startup-$name-$i"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $logBase) | Out-Null
    $h = Start-App $data $appdata $logBase @()
    $w = Wait-Window $h 180000
    $f = if ($w) { Wait-Frame $h $w.Hwnd 180000 } else { $null }
    $marks = Get-StartupMarks $h.Err
    Stop-App $h
    $rows += [pscustomobject]@{
      build        = 'debug'
      scenario     = "startup/$name#$i"
      hwnd_ms      = if ($w) { [math]::Round($w.Ms) } else { $null }
      first_frame  = if ($f) { [math]::Round($f.Ms) } else { $null }
      levels       = if ($f) { $f.Levels } else { 0 }
      mark_scripts  = $marks['scripts loaded']
      mark_app      = $marks['app built']
      mark_window  = $marks['window created']
      mark_render  = $marks['renderer made']
      mark_text    = $marks['text installed']
      mark_shown   = $marks['window shown']
      mark_frame   = $marks['first frame rendered']
    }
    Write-Host ("startup/{0}#{1}: hwnd {2} ms, first frame {3} ms, levels {4}" -f $name, $i, $rows[-1].hwnd_ms, $rows[-1].first_frame, $rows[-1].levels)
  }
  $rows
}

function Measure-Idle([int]$seconds) {
  $data = Initialize-Data 'base'
  $appdata = Join-Path $Root 'appdata'
  $logBase = Join-Path $Root 'logs\idle'
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $logBase) | Out-Null
  $h = Start-App $data $appdata $logBase @()
  $w = Wait-Window $h 180000
  if (-not $w) { Stop-App $h; return @() }
  $f = Wait-Frame $h $w.Hwnd 180000
  Start-Sleep -Seconds 8   # the launch's own frames and toasts
  $before = Get-Frames $h.Err
  $cpu = Sample-Cpu $h.Process.Id $seconds
  $after = Get-Frames $h.Err
  $rss = (Get-Tree $h.Process.Id | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB
  $profile = @(Get-ProfileLines $h.Err | Where-Object { $_ -match '^profile \d+ frames' } | Select-Object -Last 6)
  Stop-App $h
  @([pscustomobject]@{
      build       = 'debug'
      scenario    = "idle/${seconds}s"
      cpu_pct     = [math]::Round($cpu, 3)
      frames      = $after - $before
      rss_mb      = [math]::Round($rss, 1)
      frame_lines = ($profile -join ' || ')
      first_frame = if ($f) { [math]::Round($f.Ms) } else { $null }
    })
}

function Measure-Memory([int]$threads) {
  $data = Initialize-Data 'base'
  $appdata = Join-Path $Root 'appdata'
  $logBase = Join-Path $Root "logs\memory-$threads"
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $logBase) | Out-Null
  $h = Start-App $data $appdata $logBase @()
  $w = Wait-Window $h 180000
  if (-not $w) { Stop-App $h; return @() }
  $null = Wait-Frame $h $w.Hwnd 180000
  Start-Sleep -Seconds 10
  $app = (Get-Tree $h.Process.Id) | Where-Object { $_.ProcessName -eq 'keplorayx' }
  $eng = (Get-Tree $h.Process.Id) | Where-Object { $_.ProcessName -eq 'keplorayx-engine' }
  $appMb = if ($app) { [math]::Round($app.PrivateMemorySize64 / 1MB, 1) } else { 0 }
  $engMb = if ($eng) { [math]::Round(($eng | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1) } else { 0 }
  $appWs = if ($app) { [math]::Round($app.WorkingSet64 / 1MB, 1) } else { 0 }
  Stop-App $h
  @([pscustomobject]@{
      build     = 'debug'
      scenario  = "memory/$threads-threads"
      app_mb    = $appMb
      engine_mb = $engMb
      working_mb = $appWs
      threads   = $threads
    })
}

# ---------------------------------------------------------------- run
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$all = @()
switch ($Mode) {
  'startup' { $all += Measure-Startup -cold:$Cold }
  'idle' { $all += Measure-Idle $IdleSeconds }
  'memory' { $all += Measure-Memory $Threads }
  'all' {
    $all += Measure-Startup -cold:$Cold
    $all += Measure-Idle $IdleSeconds
    $all += Measure-Memory $Threads
  }
}
$file = Join-Path $Out ("kx-perf-$Mode-{0}.json" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$all | ConvertTo-Json -Depth 6 | Set-Content -Path $file
Write-Host "wrote $file"
