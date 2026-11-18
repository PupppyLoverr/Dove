<#
.SYNOPSIS
  Whether an idle Keplorayx's CPU is the frames it draws, or work behind them:
  samples the same process at rest, minimised, and at rest again.
#>
param([int]$Settle = 40, [int]$Seconds = 10, [string]$Root = "$env:USERPROFILE\.capy\work\kx-perf", [int]$Port = 27811)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class KxMin {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
}
"@
$exe = 'C:\dvw\target-kx-zgx\debug\keplorayx.exe'
$env:ZERON_ZGX_ENGINE_DATA = Join-Path $Root 'data-base\keplorayx'
$env:ZERON_ZGX_ENGINE_PORT = "$Port"
$env:APPDATA = Join-Path $Root 'appdata'
$env:KX_APP_ID = 'com.keplorayx.app.perf'
$env:ZGX_GPU_STATS = '1'
$log = Join-Path $Root 'logs\minimise'
Get-Process keplorayx, keplorayx-engine -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
$p = Start-Process -FilePath $exe -ArgumentList 'main' -WorkingDirectory (Split-Path -Parent $exe) `
  -RedirectStandardError "$log.err.txt" -RedirectStandardOutput "$log.out.txt" -PassThru
Start-Sleep -Seconds $Settle

function Sample([string]$what) {
  $a0 = (Get-Process -Id $p.Id).TotalProcessorTime.TotalMilliseconds
  $e0 = (Get-Process keplorayx-engine -ErrorAction SilentlyContinue | Measure-Object TotalProcessorTime -Sum)
  $f0 = (Select-String -Path "$log.err.txt" -Pattern 'GPU_STATS frames=(\d+)' -AllMatches |
    ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
  Start-Sleep -Seconds $Seconds
  $proc = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
  if (-not $proc) { Write-Output "$what`: process gone"; return }
  $a1 = $proc.TotalProcessorTime.TotalMilliseconds
  $f1 = (Select-String -Path "$log.err.txt" -Pattern 'GPU_STATS frames=(\d+)' -AllMatches |
    ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
  Write-Output ('{0,-12} app {1,6:N2}%  frames {2,4}  rss {3,6:N1} MB' -f $what, (($a1 - $a0) / ($Seconds * 10)), ($f1 - $f0), ($proc.PrivateMemorySize64 / 1MB))
}

$h = [IntPtr]::Zero
foreach ($w in (Get-Process -Id $p.Id).MainWindowHandle) { if ($w -ne 0) { $h = $w } }
Sample 'at rest'
if ($h -ne [IntPtr]::Zero) { [KxMin]::ShowWindow($h, 6) | Out-Null; Start-Sleep -Seconds 3; Sample 'minimised'; [KxMin]::ShowWindow($h, 9) | Out-Null; Start-Sleep -Seconds 3 }
Sample 'restored'
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Get-Process keplorayx-engine -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
