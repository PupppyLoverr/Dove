<#
.SYNOPSIS
  Keystroke to painted frame, inside the command palette.

.DESCRIPTION
  The palette's budget is p95 16.7 ms or less -- one frame at 60 Hz -- from
  the key reaching the window to the frame that shows it (reports/KEPL-13.md
  section 4.3).

  The number is the app's own, not this script's: with ZGX_INPUT_TRACE=1
  every key that reaches the screen prints

      input_trace <us> us (key ..., frame ...)

  (patches/zgx-desktop/src/application.rs). The pause between keys this
  script adds is never part of the measurement.

  Keys are posted with PostMessage, so the app takes them from its own input
  queue as it does a keyboard's. The palette is opened through devtools
  (zgx inspect ... command shell.palette) and the field emptied with
  Backspace, so a round starts at the root list.

.PARAMETER App
  The app's pid, as `zgx inspect list` reports it.

.PARAMETER Log
  The file its stderr is redirected to (this is where the trace lines are).

.PARAMETER Rounds
  How many rounds of typing to run.

.PARAMETER Text
  What is typed each round. Six to ten letters walks both the first
  keystroke (every row) and the narrowed ones (a handful).

.EXAMPLE
  powershell -File scripts\perf\kx-palette.ps1 -App 13924 -Log C:\work\err.txt
#>
param(
  [Parameter(Mandatory = $true)][int]$App,
  [Parameter(Mandatory = $true)][string]$Log,
  [int]$Rounds = 4,
  [string]$Text = 'notch',
  [int]$GapMs = 160,
  [string]$Zgx = 'C:\dvw\target-kx-zgx\debug\zgx.exe'
)
$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class KxPalette {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  public delegate bool Proc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static IntPtr FindMain(uint pid) {
    IntPtr found = IntPtr.Zero; int biggest = 0;
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      if (!IsWindowVisible(h)) return true;
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
      var sb = new StringBuilder(512); GetWindowTextW(h, sb, 512);
      if (sb.ToString() != "Keplorayx") return true;
      RECT r; GetWindowRect(h, out r);
      int area = (r.R - r.L) * (r.B - r.T);
      if (area > biggest) { biggest = area; found = h; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static void Post(IntPtr h, ushort vk, char c) {
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)1);
    PostMessage(h, 0x0102, (IntPtr)c, (IntPtr)1);
    PostMessage(h, 0x0101, (IntPtr)vk, (IntPtr)0xC0000001);
  }
  public static void Vk(IntPtr h, ushort vk) {
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)1);
    PostMessage(h, 0x0101, (IntPtr)vk, (IntPtr)0xC0000001);
  }
}
"@

$h = [KxPalette]::FindMain([uint32]$App)
if ($h -eq [IntPtr]::Zero) { Write-Error "no main window for pid $App"; exit 1 }
[KxPalette]::ShowWindow($h, 9) | Out-Null
[KxPalette]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 400

$samples = New-Object System.Collections.Generic.List[int]
for ($round = 1; $round -le $Rounds; $round++) {
  $state = (& $Zgx inspect --app $App state shell_palette_open 2>&1 | Select-Object -Last 1)
  if ($state -like '*= false*') {
    & $Zgx inspect --app $App command shell.palette | Out-Null
    Start-Sleep -Milliseconds 900
  }
  for ($i = 0; $i -lt 24; $i++) {
    [KxPalette]::Vk($h, 0x08)
    Start-Sleep -Milliseconds 10
  }
  Start-Sleep -Milliseconds 250

  $before = @(Select-String -Path $Log -Pattern 'input_trace' -ErrorAction SilentlyContinue).Count
  foreach ($c in $Text.ToCharArray()) {
    $vk = [int][char]([string]$c).ToUpper()
    [KxPalette]::Post($h, [uint16]$vk, $c)
    Start-Sleep -Milliseconds $GapMs
  }
  Start-Sleep -Milliseconds 400

  $after = @(Select-String -Path $Log -Pattern 'input_trace' -ErrorAction SilentlyContinue)
  $fresh = $after | Select-Object -Skip $before
  foreach ($line in $fresh) {
    $m = [regex]::Match($line.Line, 'input_trace (\d+) us')
    if ($m.Success) { $samples.Add([int]$m.Groups[1].Value) }
  }
}

if ($samples.Count -eq 0) { Write-Error "no input_trace lines in $Log (is ZGX_INPUT_TRACE=1 set?)"; exit 1 }
$sorted = $samples | Sort-Object
function Pct([double]$p) {
  $i = [int][math]::Floor(($p / 100.0) * ($sorted.Count - 1))
  return $sorted[$i] / 1000.0
}
"keys measured: $($samples.Count)"
"p50 {0:N2} ms" -f (Pct 50)
"p95 {0:N2} ms" -f (Pct 95)
"p99 {0:N2} ms" -f (Pct 99)
"max {0:N2} ms" -f ($sorted[$sorted.Count - 1] / 1000.0)
$bits = $sorted | ForEach-Object { "{0:N1}" -f ($_ / 1000.0) }
"raw: " + ($bits -join ", ")
