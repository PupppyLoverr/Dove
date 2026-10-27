<#
.SYNOPSIS
  Contact sheet of every notch screen (docs/NOTCH-V2.md V4): runs the island
  process once per screen on a scratch DOVE_HOME with demo data, captures the
  top of the primary display, and tiles the shots into one PNG.

.DESCRIPTION
  Uses the notch's QA hooks: DOVE_NOTCH_STATE, DOVE_NOTCH_PAGE,
  DOVE_NOTCH_DEMO=1 and DOVE_NOTCH_CAPTURABLE=1 (so screen capture can see
  the island). Never touches the real ~/.dove. -DoveHome copies a seeded
  home first (for real projects and agents); without it the demo data is
  used. Close any running island for the same home first.

.PARAMETER Motion
  Also log each screen's transition timings (DOVE_NOTCH_TIMING=1): the
  first frame, when it settled and the worst gap between frames, against
  docs/NOTCH-V2.md section 11.7. They're printed after the sheet is saved.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\notch_sheet.ps1 -Exe target-check\debug\dove.exe -Out $env:TEMP\notch-sheet.png
#>
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$DoveHome = '',
    [int]$Wait = 6,
    [switch]$Motion,
    [string[]]$Screens = @('slim:', 'alert:', 'expanded:home', 'expanded:agents', 'expanded:music', 'expanded:tray',
        'expanded:notes', 'expanded:timer', 'expanded:alerts', 'expanded:dev', 'expanded:new')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class NotchSheetDpi { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v); }
"@
[NotchSheetDpi]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
$Exe = (Resolve-Path $Exe).Path
$screen = [System.Windows.Forms.Screen]::PrimaryScreen
$bounds = $screen.Bounds
$w = [math]::Min(820, $bounds.Width); $h = 520
$dir = Join-Path $env:TEMP "dove-notch-sheet-$(Get-Date -Format yyyyMMdd-HHmmss)"
New-Item -ItemType Directory -Force $dir | Out-Null
$shots = @()
foreach ($s in $Screens) {
    $state, $page = $s.Split(':', 2)
    $home_ = Join-Path $dir "home-$state-$page"
    if ($DoveHome) { Copy-Item -Recurse $DoveHome $home_ } else { New-Item -ItemType Directory -Force $home_ | Out-Null }
    $envs = @{ DOVE_HOME = $home_; DOVE_NOTCH = '1'; DOVE_NOTCH_STATE = $state; DOVE_NOTCH_PAGE = $page; DOVE_NOTCH_DEMO = '1';
        DOVE_NOTCH_CAPTURABLE = '1'; DOVE_LOG = $(if ($Motion) { 'warn,dove::notch=info' } else { 'warn' })
        DOVE_NOTCH_TIMING = $(if ($Motion) { '1' } else { '' }) }
    $saved = @{}
    foreach ($k in $envs.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process'); [Environment]::SetEnvironmentVariable($k, $envs[$k], 'Process') }
    try {
        $p = Start-Process -FilePath $Exe -ArgumentList '--island' -PassThru -RedirectStandardOutput "$home_\out.log" -RedirectStandardError "$home_\err.log"
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k], 'Process') }
    }
    Start-Sleep -Seconds $Wait
    $file = Join-Path $dir ("{0}-{1}.png" -f $state, ($(if ($page) { $page } else { 'x' })))
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bounds.Left + ($bounds.Width - $w) / 2, $bounds.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $g.Dispose(); $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    $shots += [pscustomobject]@{ label = $s; file = $file }
}

# Tile: three across, a label over each.
$cols = 3; $rows = [math]::Ceiling($shots.Count / $cols); $label = 26
$sheet = New-Object System.Drawing.Bitmap ($w * $cols), (($h + $label) * $rows)
$g = [System.Drawing.Graphics]::FromImage($sheet)
$g.Clear([System.Drawing.Color]::FromArgb(24, 24, 27))
$font = New-Object System.Drawing.Font 'Segoe UI', 11
for ($i = 0; $i -lt $shots.Count; $i++) {
    $x = ($i % $cols) * $w; $y = [math]::Floor($i / $cols) * ($h + $label)
    $g.DrawString($shots[$i].label, $font, [System.Drawing.Brushes]::White, $x + 8, $y + 4)
    $img = [System.Drawing.Image]::FromFile($shots[$i].file)
    $g.DrawImage($img, $x, $y + $label, $w, $h); $img.Dispose()
}
$g.Dispose(); $sheet.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $sheet.Dispose()
"saved $Out (shots in $dir)"
if ($Motion) {
    foreach ($s in $Screens) {
        $state, $page = $s.Split(':', 2)
        $logs = @("home-$state-$page\out.log", "home-$state-$page\err.log") | ForEach-Object { Join-Path $dir $_ }
        Get-Content $logs -ErrorAction SilentlyContinue |
            Where-Object { $_ -match 'notch timing: (.*)$' } |
            ForEach-Object { "{0,-18} {1}" -f $s, $Matches[1] }
    }
}
