<#
.SYNOPSIS
  Quick idle check: launch Dove on a scratch DOVE_HOME, let it settle on
  Home, then print RSS, CPU and render count while it sits idle.

.DESCRIPTION
  The window opens without taking the foreground (DOVE_NO_ACTIVATE) and the
  intro is skipped. After -SettleSec it samples -SampleSec of idle:
    - RSS (working set) and private bytes of the main process, and summed
      over Dove's own child processes (WebView2, helpers),
    - CPU as % of one core for the main process,
    - renders, from DOVE_FRAME_STATS ("shell renders in the last second: N"
      lines). Idle should be 0.
  Nothing touches the real ~/.dove. For the full startup/streaming/latency
  run, see scripts/perf/perf.ps1.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\perf_check.ps1 -Exe C:\dvw\target-size\release\dove.exe

.PARAMETER Island
  Measure the notch's island process alone (dove --island: no engine, no
  window), which is what Dove costs while it's closed. The island has no
  frame stats, so idle_renders reads 0. Target (docs/NOTCH-V2.md): a small
  working set and about 0% CPU once it settles hidden.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\perf_check.ps1 -Exe target-check\debug\dove.exe -Island
#>
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [int]$SettleSec = 30,
    [int]$SampleSec = 10,
    [string]$DoveHome = "$env:TEMP\dove-perf-check-$(Get-Date -Format yyyyMMdd-HHmmss)",
    [switch]$Island
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $DoveHome | Out-Null
$outLog = Join-Path $DoveHome 'stdout.log'
$errLog = Join-Path $DoveHome 'stderr.log'

function Get-Tree([int]$RootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name
    $out = @(); $stack = New-Object System.Collections.Stack; $stack.Push($RootId)
    while ($stack.Count) {
        $id = $stack.Pop()
        foreach ($c in $all | Where-Object { $_.ParentProcessId -eq $id }) { $out += $c; $stack.Push([int]$c.ProcessId) }
    }
    $out
}

function Count-Renders([datetime]$from, [datetime]$to) {
    $total = 0
    foreach ($line in Get-Content $outLog -ErrorAction SilentlyContinue) {
        if ($line -match '^(\S+)\s+INFO dove::frames: shell renders in the last second: (\d+)') {
            $t = [datetime]::Parse($Matches[1]).ToUniversalTime()
            if ($t -ge $from.ToUniversalTime() -and $t -le $to.ToUniversalTime()) { $total += [int]$Matches[2] }
        }
    }
    $total
}

$envs = @{ DOVE_HOME = $DoveHome; DOVE_SKIP_INTRO = '1'; DOVE_NO_ACTIVATE = '1'; DOVE_FRAME_STATS = '1'; DOVE_LOG = 'warn,dove=info' }
$saved = @{}
foreach ($k in $envs.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process'); [Environment]::SetEnvironmentVariable($k, $envs[$k], 'Process') }
try {
    $start = @{ FilePath = $Exe; PassThru = $true; RedirectStandardOutput = $outLog; RedirectStandardError = $errLog }
    if ($Island) { $start.ArgumentList = '--island' }
    $p = Start-Process @start
} finally {
    foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k], 'Process') }
}

try {
    Start-Sleep -Seconds $SettleSec
    $p.Refresh()
    if ($p.HasExited) { throw "Dove exited during settle (code $($p.ExitCode)); see $errLog" }
    $from = Get-Date
    $cpu0 = $p.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Seconds $SampleSec
    $p.Refresh()
    $cpu1 = $p.TotalProcessorTime.TotalMilliseconds
    $to = Get-Date
    # A render in the window is only logged on the next one; give it a beat.
    Start-Sleep -Milliseconds 1500
    $renders = Count-Renders $from $to

    $kids = @(Get-Tree $p.Id | ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue } | Where-Object { $_ })
    $kidWs = ($kids | Measure-Object WorkingSet64 -Sum).Sum
    $kidPriv = ($kids | Measure-Object PrivateMemorySize64 -Sum).Sum
    $result = [ordered]@{
        mode              = if ($Island) { 'island' } else { 'app' }
        exe_mb            = [math]::Round((Get-Item $Exe).Length / 1MB, 2)
        main_rss_mb       = [math]::Round($p.WorkingSet64 / 1MB, 1)
        main_private_mb   = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        children          = $kids.Count
        children_names    = (($kids | Group-Object ProcessName | ForEach-Object { "$($_.Name) x$($_.Count)" }) -join ', ')
        children_rss_mb   = [math]::Round($kidWs / 1MB, 1)
        children_private_mb = [math]::Round($kidPriv / 1MB, 1)
        idle_cpu_pct      = [math]::Round(($cpu1 - $cpu0) / ($SampleSec * 10.0), 2)
        idle_renders      = $renders
        sample_sec        = $SampleSec
        settle_sec        = $SettleSec
    }
    [pscustomobject]$result | Format-List
    $result | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $DoveHome 'perf_check.json')
} finally {
    $kids = Get-Tree $p.Id
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    foreach ($k in $kids) { Stop-Process -Id $k.ProcessId -Force -ErrorAction SilentlyContinue }
}
