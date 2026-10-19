# Package a Windows release build two ways, into packaging\out:
#   Dove-Setup.exe        per-user installer (Inno Setup, no admin needed)
#   Dove-Portable.zip     unzip anywhere and run; data stays in .\data
#
# usage: powershell -File packaging\package-windows.ps1 [-ReleaseDir <dir with dove.exe>]
param([string]$ReleaseDir = "$PSScriptRoot\..\target\release")

$ErrorActionPreference = "Stop"
$ReleaseDir = (Resolve-Path $ReleaseDir).Path
$exe = Join-Path $ReleaseDir "dove.exe"
if (-not (Test-Path $exe)) { throw "No dove.exe in ${ReleaseDir}: run a release build first." }
$out = Join-Path $PSScriptRoot "out"
New-Item -ItemType Directory -Force $out | Out-Null

# The files that ship: the app, the agent-spawn helper, and DirectML.
$files = @("dove.exe", "dove-escrow.exe", "DirectML.dll") |
    ForEach-Object { Join-Path $ReleaseDir $_ } | Where-Object { Test-Path $_ }

# 1. Installer.
$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 (ISCC.exe) not found." }
& $iscc "/DDoveExe=$exe" "/O$out" (Join-Path $PSScriptRoot "dove.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)." }

# 2. Portable zip: a Dove folder with a `portable` marker beside dove.exe,
# which keeps all data in Dove\data instead of ~/.dove.
$stage = Join-Path $env:TEMP "dove-portable-$PID"
$root = Join-Path $stage "Dove"
New-Item -ItemType Directory -Force $root | Out-Null
try {
    $files | Copy-Item -Destination $root
    Set-Content -Path (Join-Path $root "portable") -Value "Dove keeps its data in the data folder beside this file. Delete this file to use ~/.dove instead." -Encoding utf8
    $zip = Join-Path $out "Dove-Portable.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path $root -DestinationPath $zip -CompressionLevel Optimal
} finally {
    Remove-Item -Recurse -Force $stage
}

Get-ChildItem $out -File | Select-Object Name, @{n = "MB"; e = { [Math]::Round($_.Length / 1MB, 1) } }, LastWriteTime
