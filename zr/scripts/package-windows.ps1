param(
    [Parameter(Mandatory)][string]$ReleasesUrl
)
# Windows packaging for the app this branch ships. Two binaries go in, side by
# side because the app starts the engine from its own folder
# (apps/zgx/src/engine/manager.rs):
#   keplorayx         apps/zgx — the interface, and `zeron`'s command line
#   keplorayx-engine  zr       — the background engine it starts and attaches to
# and the interface's scripts (`ui/`, `plugins-builtin/`) beside them, which is
# where apps/zgx/src/main.rs's ui_dir() reads them from in an install
# (`CARGO_MANIFEST_DIR` is the machine that built it, not the one it runs on).
#
# Usage: scripts/package-windows.ps1 -ReleasesUrl https://…
# Env:   $env:CARGO_TARGET_DIR to build outside both workspaces' target dirs.
$ErrorActionPreference = 'Stop'
if (-not $ReleasesUrl.StartsWith('https://')) { throw 'Release feed must use HTTPS' }
$root = Split-Path $PSScriptRoot -Parent

function Find-InnoSetupCompiler {
    # Inno Setup 6 ships on GitHub's Windows runners; locally, install it with
    # `winget install JRSoftware.InnoSetup`.
    $command = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $bases = @(${env:ProgramFiles(x86)}, $env:ProgramFiles, "$env:LOCALAPPDATA/Programs") |
        Where-Object { $_ }
    foreach ($base in $bases) {
        $candidate = Join-Path $base 'Inno Setup 6/ISCC.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    throw 'Inno Setup 6 (ISCC.exe) is required to build the installer: winget install JRSoftware.InnoSetup'
}

function Get-WindowsPackageArch([string]$Path) {
    # Match zeron-update's `std::env::consts::ARCH` so the standalone .exe
    # name agrees with crates/update/src/windows.rs::artifact. Read the built
    # executable's PE machine type rather than this PowerShell process's
    # architecture: x64 PowerShell under ARM64 emulation reports X64 no matter
    # which toolchain rustc used.
    $stream = [IO.File]::OpenRead($Path)
    try {
        $reader = [IO.BinaryReader]::new($stream)
        $stream.Position = 0x3C
        $stream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x4550) { throw "Not a PE executable: $Path" }
        $machine = $reader.ReadUInt16()
    } finally { $stream.Dispose() }
    switch ($machine) {
        0x8664 { 'x86_64' }
        0xAA64 { 'aarch64' }
        default { throw ('Unsupported Windows executable machine type: 0x{0:X4}' -f $machine) }
    }
}

Push-Location $root
try {
    # Each workspace builds its own way: the interface is a workspace of its
    # own (its own lock and patches), the engine is a bin of `zr`'s.
    $appTarget = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'apps/zgx/target' }
    $engineTarget = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
    $appBin = Join-Path $appTarget 'release/keplorayx.exe'
    $engineBin = Join-Path $engineTarget 'release/keplorayx-engine.exe'
    cargo build --release --locked --target-dir $engineTarget -p zeron --bin keplorayx-engine
    if ($LASTEXITCODE -ne 0) { throw 'Engine build failed' }
    Push-Location (Join-Path $root 'apps/zgx')
    try {
        cargo build --release --locked --target-dir $appTarget
        if ($LASTEXITCODE -ne 0) { throw 'Interface build failed' }
    } finally { Pop-Location }
    foreach ($bin in $appBin, $engineBin) {
        if (-not (Test-Path -LiteralPath $bin)) { throw "No binary at $bin" }
    }
    # Explicit pipes also work for the GUI-subsystem executable in CI. A
    # PowerShell collection match does not populate the scalar $Matches map.
    $probe = [Diagnostics.ProcessStartInfo]::new()
    $probe.FileName = (Resolve-Path -LiteralPath $appBin).Path
    $probe.Arguments = '--version'
    $probe.UseShellExecute = $false
    $probe.CreateNoWindow = $true
    $probe.RedirectStandardOutput = $true
    $probe.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($probe)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(10000)) {
            $process.Kill()
            throw 'Executable version probe timed out'
        }
        # `zeron <version>`: crates/cli's command, which keeps that name
        # whatever the binary is called (src/cli tests/golden assert it).
        $versionMatch = [regex]::Match($stdout.Result.Trim(), '\Azeron (\d+\.\d+\.\d+(-beta\.\d+)?)\z')
        if ($process.ExitCode -ne 0 -or -not $versionMatch.Success) {
            throw "Cannot read executable version: $($stderr.Result)"
        }
        $version = $versionMatch.Groups[1].Value
    } finally { $process.Dispose() }
    $out = Join-Path $root 'target/package'
    $arch = Get-WindowsPackageArch $probe.FileName
    $stage = Join-Path $out "zeron-$version-windows-$arch"
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item -LiteralPath $appBin -Destination (Join-Path $stage 'keplorayx.exe')
    Copy-Item -LiteralPath $engineBin -Destination (Join-Path $stage 'keplorayx-engine.exe')
    # The interface's scripts, beside the executable: ui_dir() reads these
    # from its own folder in an install, and plugins from the sibling folder.
    $ui = Join-Path $stage 'ui'
    $plugins = Join-Path $stage 'plugins-builtin'
    Copy-Item -LiteralPath (Join-Path $root 'apps/zgx/ui') -Destination $ui -Recurse
    if (Test-Path -LiteralPath (Join-Path $root 'apps/zgx/plugins-builtin')) {
        Copy-Item -LiteralPath (Join-Path $root 'apps/zgx/plugins-builtin') -Destination $plugins -Recurse
    }
    foreach ($junk in '.zgx', 'tests') {
        $p = Join-Path $ui $junk
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force }
    }
    @{ releases_url = $ReleasesUrl } | ConvertTo-Json | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $stage 'zeron-update.json')
    Copy-Item -LiteralPath 'LICENSE','THIRD_PARTY_NOTICES.md' -Destination $stage
    $licenses = Join-Path $stage 'licenses/fonts'
    New-Item -ItemType Directory -Force -Path $licenses | Out-Null
    Copy-Item -Path 'crates/ui/assets/fonts/licenses/*' -Destination $licenses
    Copy-Item -Path 'apps/zgx/fonts/*-OFL.txt' -Destination $licenses
    Copy-Item -LiteralPath 'crates/voice/NOTICE.md' -Destination (Join-Path $stage 'licenses/parakeet-v3.txt')
    Compress-Archive -Path "$stage/*" -DestinationPath "$stage.zip" -Force
    # The updater's standalone artifact is the interface alone (crates/update
    # `artifact`): the engine beside it is what makes that directory work, so
    # an in-app update that keeps the old engine is the intended shape.
    Copy-Item -LiteralPath $appBin -Destination "$stage.exe"
    # The per-user installer wraps the same staged directory (zeron-update.json
    # included), so installed copies update in place like the portable zip.
    $iscc = Find-InnoSetupCompiler
    & $iscc /Qp "/DAppVersion=$version" "/DArch=$arch" `
        "/DAppExe=keplorayx.exe" `
        "/DPackageDir=$([IO.Path]::GetFullPath($stage))" `
        "/DOutputDir=$([IO.Path]::GetFullPath($out))" `
        ([IO.Path]::GetFullPath((Join-Path $root 'dist/windows/zeron.iss')))
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
    $setup = Join-Path $out "zeron-$version-windows-$arch-setup.exe"
    if (-not (Test-Path -LiteralPath $setup)) { throw "Installer not produced: $setup" }
    $file = Split-Path "$stage.exe" -Leaf
    $hash = (Get-FileHash -LiteralPath "$stage.exe" -Algorithm SHA256).Hash.ToLowerInvariant()
    @{ version = $version; files = @{ $file = @{ sha256 = $hash } } } |
        ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $out 'manifest.json')
    Write-Host "Packaged $stage.zip and $setup"
} finally { Pop-Location }
