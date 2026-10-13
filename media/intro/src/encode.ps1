# Encodes the captured frames (../frames) into the shareable intro videos,
# then fails if any output is over 1 MB. Run `npm run assets` and `npm run capture` first.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$frames = Join-Path $root 'frames\f%04d.png'
$out = $root
$fps = 30
Push-Location $root
try {
  # WebM: VP9 two-pass.
  ffmpeg -loglevel error -y -framerate $fps -i $frames -c:v libvpx-vp9 -b:v 0 -crf 38 -row-mt 1 -pix_fmt yuv420p -pass 1 -an -f null NUL
  ffmpeg -loglevel error -y -framerate $fps -i $frames -c:v libvpx-vp9 -b:v 0 -crf 38 -row-mt 1 -pix_fmt yuv420p -pass 2 `
    -an (Join-Path $out 'dove-intro.webm')
  Remove-Item ffmpeg2pass-0.log -ErrorAction SilentlyContinue

  # MP4: H.264, plays everywhere.
  ffmpeg -loglevel error -y -framerate $fps -i $frames -c:v libx264 -preset slow -tune animation -crf 26 -pix_fmt yuv420p `
    -an -movflags +faststart (Join-Path $out 'dove-intro.mp4')

  # Animated WebP for the README (loops).
  ffmpeg -loglevel error -y -framerate $fps -i $frames -vf "fps=20,scale=960:-1:flags=lanczos" -c:v libwebp_anim -q:v 60 -loop 0 `
    (Join-Path $out 'dove-intro.webp')

  # Poster: the finished wordmark.
  $last = (Get-ChildItem (Join-Path $root 'frames\f*.png') | Sort-Object Name | Select-Object -Last 1).FullName
  Copy-Item $last (Join-Path $out 'poster.png') -Force

  $over = $false
  foreach ($f in 'dove-intro.webm', 'dove-intro.mp4', 'dove-intro.webp', 'poster.png') {
    $size = (Get-Item (Join-Path $out $f)).Length
    $flag = if ($size -gt 1MB) { $over = $true; '  OVER 1 MB' } else { '' }
    '{0,-16} {1,8:N0} KB{2}' -f $f, ($size / 1KB), $flag
  }
  if ($over) { throw 'an output is over 1 MB' }
} finally {
  Pop-Location
}
