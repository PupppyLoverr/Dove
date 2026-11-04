# The design PNGs: every gallery page (and each fixture given) in night
# and dawn, into apps/keplorayx/docs/design/shots/.
#
#   powershell -File apps\keplorayx\tools\shots.ps1
#   powershell -File apps\keplorayx\tools\shots.ps1 -Pages icons,buttons -Themes orbit-night
param(
    [string[]]$Pages = @("tokens", "type", "icons", "buttons", "controls", "rows", "surfaces", "overlays", "motion"),
    [string[]]$Fixtures = @(),
    [string[]]$Themes = @("orbit-night", "orbit-dawn"),
    [int]$Seconds = 4
)
$Pages = @($Pages | ForEach-Object { $_ -split "," } | Where-Object { $_ })
$Fixtures = @($Fixtures | ForEach-Object { $_ -split "," } | Where-Object { $_ })
$Themes = @($Themes | ForEach-Object { $_ -split "," } | Where-Object { $_ })
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$out = Join-Path $here "..\docs\design\shots"
foreach ($theme in $Themes) {
    $tag = $theme -replace 'orbit-', ''
    foreach ($page in $Pages) {
        & powershell -ExecutionPolicy Bypass -File (Join-Path $here "shot.ps1") -Mode gallery -Page $page -Theme $theme -Seconds $Seconds -Out (Join-Path $out "gallery-$page-$tag.png")
    }
    foreach ($fixture in $Fixtures) {
        & powershell -ExecutionPolicy Bypass -File (Join-Path $here "shot.ps1") -Mode main -Fixture $fixture -Theme $theme -Seconds $Seconds -Out (Join-Path $out "$fixture-$tag.png")
    }
}
