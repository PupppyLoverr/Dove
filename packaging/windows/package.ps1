param(
    # Folder holding the release binaries: dove.exe, and when built,
    # dove-escrow.exe and dove-mesh-host.exe.
    [string]$Bin = "target\release",
    [string]$MeshBin = "mesh\target\release",
    [string]$Out = "packaging\out"
)
# Windows packages: Dove-Setup-<version>.exe (Inno Setup, per-user install)
# and Dove-Portable-<version>-x64.zip (unzip and run; no install, no admin).
$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\..\.."
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = Resolve-Path $Out

$stage = Join-Path $Out "Dove"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item "$Bin\dove.exe" $stage
# Found beside dove.exe at run time: the agent-spawn helper, DirectML
# (ONNX Runtime on Windows) and the remote-control host.
foreach ($f in @("$Bin\dove-escrow.exe", "$Bin\DirectML.dll", "$MeshBin\dove-mesh-host.exe")) {
    if (Test-Path $f) { Copy-Item $f $stage } else { Write-Warning "missing $f (skipped)" }
}

# Portable: the same files plus a `portable` marker, which keeps all of
# Dove's data in a `data` folder beside dove.exe instead of ~/.dove (an
# explicit DOVE_HOME still wins). The installer gets no marker.
$portable = Join-Path $Out "Dove-Portable"
if (Test-Path $portable) { Remove-Item -Recurse -Force $portable }
New-Item -ItemType Directory -Force $portable | Out-Null
Copy-Item "$stage\*" $portable
Set-Content -Path (Join-Path $portable "portable") -Encoding utf8 -Value "Dove keeps its data in the data folder beside this file. Delete this file to use ~/.dove instead."
$zip = Join-Path $Out "Dove-Portable-$version-x64.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $portable -DestinationPath $zip
Write-Host "portable: $zip"

$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source }
if (-not $iscc) { throw "Inno Setup 6 (ISCC.exe) not found" }
& $iscc "/DStage=$stage" "/DAppVersion=$version" "/O$Out" "/FDove-Setup-$version" "packaging\dove.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
Write-Host "setup: $(Join-Path $Out "Dove-Setup-$version.exe")"
