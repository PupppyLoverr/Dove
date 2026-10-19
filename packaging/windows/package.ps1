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
# Optional sidecars, found beside dove.exe at run time.
foreach ($f in @("$Bin\dove-escrow.exe", "$MeshBin\dove-mesh-host.exe")) {
    if (Test-Path $f) { Copy-Item $f $stage } else { Write-Warning "missing $f (skipped)" }
}

# Portable: a marker file beside dove.exe; the binaries are the same.
"Dove portable build $version. Run dove.exe. Settings and threads live in %USERPROFILE%\.dove unless DOVE_HOME is set." |
    Out-File -Encoding utf8 (Join-Path $stage "README.txt")
$zip = Join-Path $Out "Dove-Portable-$version-x64.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Write-Host "portable: $zip"

$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source }
if (-not $iscc) { throw "Inno Setup 6 (ISCC.exe) not found" }
& $iscc "/DStage=$stage" "/DAppVersion=$version" "/O$Out" "/FDove-Setup-$version" "packaging\dove.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
Write-Host "setup: $(Join-Path $Out "Dove-Setup-$version.exe")"
