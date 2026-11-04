# Launch keplorayx for a few seconds against a scratch DOVE_HOME and print
# what its scripts reported (load errors, design-lint warnings).
#
#   powershell -File apps\keplorayx\tools\smoke.ps1 -Mode gallery
#   powershell -File apps\keplorayx\tools\smoke.ps1 -Mode main -Seconds 15
param(
    [string]$Mode = "main",
    [int]$Seconds = 10,
    [string]$Exe = "C:\dvw\target-kx-zgx\debug\keplorayx.exe",
    [string]$DataHome = "$env:TEMP\kx-scratch-home",
    [string[]]$Extra = @()
)
New-Item -ItemType Directory -Force $DataHome | Out-Null
$env:DOVE_HOME = $DataHome
if (-not $env:ZGX_DESIGN_LINT) { $env:ZGX_DESIGN_LINT = "warn" }
$err = "$env:TEMP\kx-smoke-$Mode.err"
$out = "$env:TEMP\kx-smoke-$Mode.out"
$argv = @($Mode) + $Extra
$p = Start-Process -FilePath $Exe -ArgumentList $argv -PassThru -RedirectStandardError $err -RedirectStandardOutput $out
Start-Sleep -Seconds $Seconds
$alive = -not $p.HasExited
if ($alive) {
    $p.Refresh()
    "alive: yes  working set: {0:N0} MB  private: {1:N0} MB  cpu: {2:N1} s" -f ($p.WorkingSet64 / 1MB), ($p.PrivateMemorySize64 / 1MB), $p.TotalProcessorTime.TotalSeconds
    Stop-Process -Id $p.Id -Force
} else {
    "alive: no (exit $($p.ExitCode))"
}
Get-Content $err | Select-Object -First 60
