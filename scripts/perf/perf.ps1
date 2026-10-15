<#
.SYNOPSIS
  Dove performance harness: startup, idle, streaming (1/5/10 concurrent
  sessions with the fake agent), input latency and RAM split by process class.

.DESCRIPTION
  For each session count it:
    1. builds a scratch DOVE_HOME (a short launch creates the schema, then
       seed_home.py adds a git project, fake-agent paths and N automations),
    2. launches Dove without taking the foreground (DOVE_NO_ACTIVATE),
       timing process start -> window visible -> first frame,
    3. samples idle CPU and renders,
    4. waits for the scheduler's first tick (about 60 s) to start N sessions,
    5. samples CPU and RAM while all N stream, and posts key presses to the
       window to measure keypress -> frame latency,
    6. samples idle again after the sessions end, then closes Dove.
  Nothing touches the real ~/.dove. Results go to <OutDir>\results.json and a
  markdown summary.

  Caveat: the window is visible but not focused, and GPUI caps an inactive
  window at about 30 fps while it animates. Pass -Activate (it will take the
  foreground for the whole run) to measure a focused window.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\perf\perf.ps1 -Exe C:\dvw\target-perf\release\dove.exe -Sessions 1,5,10
#>
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$FakeAgent = "$PSScriptRoot\fake-agent\target\release\dove-fake-agent.exe",
    [int[]]$Sessions = @(1, 5, 10),
    [string]$OutDir = "$env:TEMP\dove-perf-$(Get-Date -Format yyyyMMdd-HHmmss)",
    [int]$IdleSec = 15,
    [int]$StreamSampleSec = 30,
    [int]$Keys = 40,
    [switch]$Activate,
    # Keep the machine's PATH (real agent CLIs are then detected, and the
    # startup model-catalog refresh runs). Default: a minimal PATH, so only
    # the fake agent runs and numbers are repeatable.
    [switch]$RealPath
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$FakeAgent = (Resolve-Path $FakeAgent).Path
New-Item -ItemType Directory -Force $OutDir | Out-Null

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class PerfWin {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public static IntPtr Find(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) { found = h; return false; } return true; }, IntPtr.Zero);
    return found;
  }
}
"@

function Get-Tree([int]$RootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, CommandLine
    $byParent = @{}
    foreach ($p in $all) { $k = [int]$p.ParentProcessId; if (-not $byParent[$k]) { $byParent[$k] = @() }; $byParent[$k] += $p }
    $out = @(); $stack = New-Object System.Collections.Stack; $stack.Push($RootId)
    while ($stack.Count) {
        $id = $stack.Pop()
        foreach ($c in @($byParent[$id])) { if ($c) { $out += $c; $stack.Push([int]$c.ProcessId) } }
    }
    $out
}

# Dove's own processes vs the agents it runs (reported separately).
function Get-Class($proc, [string]$exeName) {
    $n = $proc.Name.ToLowerInvariant(); $cl = "$($proc.CommandLine)"
    if ($n -eq $exeName) { if ($cl -match '--mcp-browser') { return 'dove-mcp-bridge' } else { return 'dove-helper' } }
    if ($n -eq 'msedgewebview2.exe') { return 'dove-webview' }
    if ($n -like 'dove-mesh-host*') { return 'dove-mesh-host' }
    if ($n -eq 'conhost.exe') { return 'console' }
    if ($n -like 'dove-fake-agent*') { return 'harness:fake-agent' }
    return "harness:$n"
}

function Sample-Processes([int]$RootId, [string]$exeName) {
    $rows = @()
    $main = Get-Process -Id $RootId -ErrorAction SilentlyContinue
    if ($main) { $rows += [pscustomobject]@{ Id = $RootId; Class = 'dove-main'; Private = $main.PrivateMemorySize64; WS = $main.WorkingSet64; Cpu = $main.TotalProcessorTime.TotalMilliseconds } }
    foreach ($c in Get-Tree $RootId) {
        $p = Get-Process -Id $c.ProcessId -ErrorAction SilentlyContinue
        if ($p) { $rows += [pscustomobject]@{ Id = [int]$c.ProcessId; Class = (Get-Class $c $exeName); Private = $p.PrivateMemorySize64; WS = $p.WorkingSet64; Cpu = $p.TotalProcessorTime.TotalMilliseconds } }
    }
    $rows
}

function Summarize-Memory($rows) {
    $own = $rows | Where-Object { $_.Class -like 'dove-*' }
    $harness = $rows | Where-Object { $_.Class -like 'harness:*' }
    $byClass = @{}
    foreach ($g in ($rows | Group-Object Class)) {
        $byClass[$g.Name] = [ordered]@{ count = $g.Count; private_mb = [math]::Round(($g.Group | Measure-Object Private -Sum).Sum / 1MB, 1); ws_mb = [math]::Round(($g.Group | Measure-Object WS -Sum).Sum / 1MB, 1) }
    }
    [ordered]@{
        dove_own_private_mb = [math]::Round(($own | Measure-Object Private -Sum).Sum / 1MB, 1)
        dove_own_ws_mb      = [math]::Round(($own | Measure-Object WS -Sum).Sum / 1MB, 1)
        harness_private_mb  = [math]::Round(($harness | Measure-Object Private -Sum).Sum / 1MB, 1)
        harness_ws_mb       = [math]::Round(($harness | Measure-Object WS -Sum).Sum / 1MB, 1)
        by_class            = $byClass
    }
}

# CPU % of one core per class between two samples (processes in both).
function Cpu-Between($a, $b, [double]$sec) {
    $prev = @{}; foreach ($r in $a) { $prev[$r.Id] = $r }
    $res = @{}
    foreach ($r in $b) {
        if ($prev.ContainsKey($r.Id)) {
            $d = ($r.Cpu - $prev[$r.Id].Cpu) / ($sec * 10.0)
            $k = if ($r.Class -like 'harness:*') { 'harness' } elseif ($r.Class -eq 'dove-main') { 'dove_main' } else { 'dove_other' }
            $res[$k] = [math]::Round(($res[$k] + $d), 2)
        }
    }
    $res
}

function Count-Renders([string]$log, [datetime]$from, [datetime]$to) {
    # tracing lines: "2026-10-15T20:01:02.123456Z  INFO dove::frames: shell renders in the last second: 3"
    $total = 0
    foreach ($line in Get-Content $log -ErrorAction SilentlyContinue) {
        if ($line -match '^(\S+)\s+INFO dove::frames: shell renders in the last second: (\d+)') {
            $t = [datetime]::Parse($Matches[1]).ToUniversalTime()
            if ($t -ge $from.ToUniversalTime() -and $t -le $to.ToUniversalTime()) { $total += [int]$Matches[2] }
        }
    }
    $total
}

function Parse-Duration([string]$s) {
    if ($s -match '([\d.]+)(ns|µs|us|ms|s)$') {
        $v = [double]$Matches[1]
        switch ($Matches[2]) { 'ns' { return $v / 1e6 } 'µs' { return $v / 1e3 } 'us' { return $v / 1e3 } 'ms' { return $v } 's' { return $v * 1e3 } }
    }
    $null
}

function Input-Latency([string]$log) {
    # "dove-input: ... to_frame_start_ms=X" followed by GPUI's "frame duration: Y".
    $lines = Get-Content $log -ErrorAction SilentlyContinue
    $lat = @(); $pending = $null
    foreach ($line in $lines) {
        if ($line -match 'dove-input: .*to_frame_start_ms=([\d.]+)') {
            if ($null -ne $pending) { $lat += $pending }
            $pending = [double]$Matches[1]
        } elseif ($null -ne $pending -and $line -match '^frame duration: (\S+)') {
            $d = Parse-Duration $Matches[1]; if ($null -ne $d) { $lat += ($pending + $d) }; $pending = $null
        }
    }
    if ($lat.Count -eq 0) { return $null }
    $sorted = $lat | Sort-Object
    [ordered]@{ samples = $sorted.Count; p50_ms = [math]::Round($sorted[[int][math]::Floor(($sorted.Count - 1) * 0.5)], 2); p95_ms = [math]::Round($sorted[[int][math]::Floor(($sorted.Count - 1) * 0.95)], 2); p99_ms = [math]::Round($sorted[[int][math]::Floor(($sorted.Count - 1) * 0.99)], 2); max_ms = [math]::Round($sorted[-1], 2) }
}

function Post-Keys([IntPtr]$hwnd, [int]$count) {
    # 'a' down/up, 150 ms apart: GPUI translates the key-down into a character.
    for ($i = 0; $i -lt $count; $i++) {
        [PerfWin]::PostMessage($hwnd, 0x0100, [IntPtr]0x41, [IntPtr]0x001E0001) | Out-Null
        Start-Sleep -Milliseconds 30
        [PerfWin]::PostMessage($hwnd, 0x0101, [IntPtr]0x41, [IntPtr]0xC01E0001) | Out-Null
        Start-Sleep -Milliseconds 120
    }
}

function Start-Dove([string]$doveHome, [string]$errLog, [string]$outLog) {
    # Start-Process redirects both pipes to files (no blocking), and the child
    # inherits these variables, which are restored right after the launch.
    $envs = @{
        DOVE_HOME = $doveHome; DOVE_SKIP_INTRO = '1'; DOVE_NO_ACTIVATE = $(if ($Activate) { '' } else { '1' })
        DOVE_STARTUP_TRACE = '1'; DOVE_INPUT_TRACE = '1'; DOVE_FRAME_STATS = '1'; ZED_MEASUREMENTS = '1'
        DOVE_WINDOW_TITLE = 'Dove perf'; DOVE_LOG = 'warn,dove=info'
        FAKE_AGENT_SECS = '150'; FAKE_AGENT_CPS = '240'; FAKE_AGENT_CHUNK = '12'; FAKE_AGENT_TOOL_EVERY = '6'
    }
    if (-not $RealPath) { $envs['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot;$(Split-Path (Get-Command git).Source)" }
    $saved = @{}
    foreach ($k in $envs.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process'); [Environment]::SetEnvironmentVariable($k, $envs[$k], 'Process') }
    try {
        $p = Start-Process -FilePath $Exe -PassThru -RedirectStandardError $errLog -RedirectStandardOutput $outLog
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k], 'Process') }
    }
    $p
}

function Stop-Tree([int]$RootId) {
    $kids = Get-Tree $RootId
    Stop-Process -Id $RootId -Force -ErrorAction SilentlyContinue
    foreach ($k in $kids) { Stop-Process -Id $k.ProcessId -Force -ErrorAction SilentlyContinue }
}

$exeName = [IO.Path]::GetFileName($Exe).ToLowerInvariant()
$results = [ordered]@{ exe = $Exe; started = (Get-Date).ToString('s'); focused = [bool]$Activate; real_path = [bool]$RealPath; runs = @() }

foreach ($n in $Sessions) {
    Write-Host "== $n session(s)"
    $run = Join-Path $OutDir "s$n"
    $doveHome = Join-Path $run 'home'; $proj = Join-Path $run 'project'
    New-Item -ItemType Directory -Force $doveHome, $proj | Out-Null
    # A small git project for the sessions to work in.
    Set-Content -Path (Join-Path $proj 'README.md') -Value "# perf project" -Encoding utf8
    git -C $proj init -q 2>$null; git -C $proj add -A 2>$null; git -C $proj -c user.email=perf@dove -c user.name=perf commit -qm init 2>$null

    # 1. Schema: one short launch creates dove.db.
    $p = Start-Dove $doveHome (Join-Path $run 'schema.err.log') (Join-Path $run 'schema.out.log')
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path (Join-Path $doveHome 'dove.db')) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
    Start-Sleep -Seconds 3; Stop-Tree $p.Id; Start-Sleep -Seconds 1
    python -I "$PSScriptRoot\seed_home.py" $doveHome $proj $FakeAgent $n | Write-Host

    # 2. Launch and time startup.
    $errLog = Join-Path $run 'dove.err.log'
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Dove $doveHome $errLog (Join-Path $run 'dove.out.log')
    $launch = Get-Date
    $hwnd = [IntPtr]::Zero
    while ($hwnd -eq [IntPtr]::Zero -and $sw.ElapsedMilliseconds -lt 30000) { $hwnd = [PerfWin]::Find([uint32]$p.Id); Start-Sleep -Milliseconds 5 }
    $windowMs = $sw.ElapsedMilliseconds
    if ($Activate) { [PerfWin]::SetForegroundWindow($hwnd) | Out-Null }
    $startupLine = $null
    while (-not $startupLine -and $sw.ElapsedMilliseconds -lt 30000) { Start-Sleep -Milliseconds 100; $startupLine = Select-String -Path $errLog -Pattern 'dove-startup:' -ErrorAction SilentlyContinue | Select-Object -First 1 }
    $firstFrameMs = $null
    if ($startupLine -and $startupLine.Line -match 'main_unix_ms=(\d+) first_render_ms=([\d.]+)') {
        $procStartUnix = ([DateTimeOffset]$p.StartTime).ToUnixTimeMilliseconds()
        $firstFrameMs = [math]::Round(([double]$Matches[1] - $procStartUnix) + [double]$Matches[2], 1)
    }

    # 3. Idle, after the launch work and the caret's blink have settled.
    Start-Sleep -Seconds 20
    $a = Sample-Processes $p.Id $exeName; $t0 = Get-Date
    Start-Sleep -Seconds $IdleSec
    $b = Sample-Processes $p.Id $exeName; $t1 = Get-Date
    $idle = [ordered]@{ cpu = (Cpu-Between $a $b $IdleSec); renders_per_sec = [math]::Round((Count-Renders $errLog $t0 $t1) / $IdleSec, 2); memory = (Summarize-Memory $b) }

    # 4. Sessions start on the scheduler's first tick (60 s after launch).
    $deadline = $launch.AddSeconds(100)
    do { Start-Sleep -Seconds 1; $agents = @(Get-Tree $p.Id | Where-Object { $_.Name -like 'dove-fake-agent*' }) } while ($agents.Count -lt $n -and (Get-Date) -lt $deadline)
    $started = $agents.Count
    Start-Sleep -Seconds 5

    # 5. Streaming: CPU/RAM sample with key presses in the middle.
    $a = Sample-Processes $p.Id $exeName; $t0 = Get-Date
    Start-Sleep -Seconds 5
    Post-Keys $hwnd $Keys
    $rest = $StreamSampleSec - ((Get-Date) - $t0).TotalSeconds
    if ($rest -gt 0) { Start-Sleep -Milliseconds ([int]($rest * 1000)) }
    $b = Sample-Processes $p.Id $exeName; $t1 = Get-Date
    $streamSec = ($t1 - $t0).TotalSeconds
    $stream = [ordered]@{ sessions_started = $started; cpu = (Cpu-Between $a $b $streamSec); renders_per_sec = [math]::Round((Count-Renders $errLog $t0 $t1) / $streamSec, 2); memory = (Summarize-Memory $b) }

    # 6. After the turns end (FAKE_AGENT_SECS = 150): idle again.
    # Turns last FAKE_AGENT_SECS (150 s) from the first tick (~60 s after launch).
    $until = $launch.AddSeconds(60 + 160)
    while ((Get-Date) -lt $until) { Start-Sleep -Seconds 2 }
    Start-Sleep -Seconds 10
    $a = Sample-Processes $p.Id $exeName; $t0 = Get-Date
    Start-Sleep -Seconds $IdleSec
    $b = Sample-Processes $p.Id $exeName; $t1 = Get-Date
    $after = [ordered]@{ cpu = (Cpu-Between $a $b $IdleSec); renders_per_sec = [math]::Round((Count-Renders $errLog $t0 $t1) / $IdleSec, 2); memory = (Summarize-Memory $b) }

    Stop-Tree $p.Id
    Start-Sleep -Seconds 2
    $results.runs += [ordered]@{
        sessions = $n; startup = [ordered]@{ window_ms = $windowMs; first_frame_ms = $firstFrameMs }
        idle = $idle; streaming = $stream; input_latency = (Input-Latency $errLog); after_sessions = $after
    }
    $results | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $OutDir 'results.json') -Encoding utf8
}

# Markdown summary.
$md = @("# Dove perf run $($results.started)", "", "exe: ``$Exe``  focused: $($results.focused)  real PATH: $($results.real_path)", "",
    "| Sessions | Window (ms) | First frame (ms) | Idle CPU main % | Idle renders/s | Stream CPU main % | Stream CPU Dove other % | Stream renders/s | Dove own private MB (stream) | Harness private MB (stream) | Key->frame p50 / p99 (ms) | After CPU main % | After renders/s |",
    "|---|---|---|---|---|---|---|---|---|---|---|---|---|")
foreach ($r in $results.runs) {
    $lat = if ($r.input_latency) { "$($r.input_latency.p50_ms) / $($r.input_latency.p99_ms)" } else { 'n/a' }
    $md += "| $($r.sessions) ($($r.streaming.sessions_started) started) | $($r.startup.window_ms) | $($r.startup.first_frame_ms) | $($r.idle.cpu.dove_main) | $($r.idle.renders_per_sec) | $($r.streaming.cpu.dove_main) | $($r.streaming.cpu.dove_other) | $($r.streaming.renders_per_sec) | $($r.streaming.memory.dove_own_private_mb) | $($r.streaming.memory.harness_private_mb) | $lat | $($r.after_sessions.cpu.dove_main) | $($r.after_sessions.renders_per_sec) |"
}
$md | Set-Content -Path (Join-Path $OutDir 'summary.md') -Encoding utf8
$md | Write-Host
Write-Host "results: $OutDir"
