<#
.SYNOPSIS
  Where an idle Keplorayx spends its CPU: per-thread over a sample, and whether
  frames were drawn in it.
#>
param([int]$Seconds = 10, [string]$Root = "$env:USERPROFILE\.capy\work\kx-perf", [int]$Port = 27811)
$ErrorActionPreference = 'Stop'
$exe = 'C:\dvw\target-kx-zgx\debug\keplorayx.exe'
$data = Join-Path $Root 'data-base\keplorayx'
$appdata = Join-Path $Root 'appdata'
$env:ZERON_ZGX_ENGINE_DATA = $data
$env:ZERON_ZGX_ENGINE_PORT = "$Port"
$env:APPDATA = $appdata
$env:KX_APP_ID = 'com.keplorayx.app.perf'
$env:ZGX_GPU_STATS = '1'
$log = Join-Path $Root 'logs\threads'
$p = Start-Process -FilePath $exe -ArgumentList 'main' -WorkingDirectory (Split-Path -Parent $exe) `
  -RedirectStandardError "$log.err.txt" -RedirectStandardOutput "$log.out.txt" -PassThru
Start-Sleep -Seconds 25

$before = @{}
foreach ($t in $p.Threads) { $before[$t.Id] = @{ Cpu = $t.TotalProcessorTime; Name = $t.ThreadState } }
$cpuBefore = $p.TotalProcessorTime
$framesBefore = (Select-String -Path "$log.err.txt" -Pattern 'GPU_STATS frames=(\d+)' -AllMatches |
  ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
Start-Sleep -Seconds $Seconds
$p.Refresh()
$cpuAfter = $p.TotalProcessorTime
$framesAfter = (Select-String -Path "$log.err.txt" -Pattern 'GPU_STATS frames=(\d+)' -AllMatches |
  ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum

"app cpu: {0:N2}% of one core over {1}s" -f (($cpuAfter - $cpuBefore).TotalMilliseconds / ($Seconds * 10)), $Seconds
"frames drawn in the sample: {0}" -f ($framesAfter - $framesBefore)
"private MB: {0:N1}" -f ($p.PrivateMemorySize64 / 1MB)
foreach ($t in $p.Threads) {
  $was = $before[$t.Id]
  if (-not $was) { continue }
  $used = ($t.TotalProcessorTime - $was.Cpu).TotalMilliseconds
  if ($used -ge 20) { "  thread {0,-7} {1,8:N0} ms" -f $t.Id, $used }
}
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Get-Process keplorayx-engine -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
