<#
.SYNOPSIS
  Ctrl+1–9 through Keplorayx's own input path, and the app's own key-to-frame
  number for each switch (`ZGX_INPUT_TRACE=1`).

.DESCRIPTION
  A switch is a key, not a click: these are posted as WM_KEYDOWN/WM_KEYUP pairs
  with Ctrl held around them, which is the path the shell's ⌘1–9 command takes.
  Each one prints `input_trace <us> us (key …, frame …)` when the frame that
  shows it is drawn, so the numbers are the app's own.

.PARAMETER Cycles
  How many times to walk 1..9 (9 switches a cycle).

.PARAMETER GapMs
  Between keys. A switch that has not shown yet is not idle, so the gap only
  has to clear the previous frame.
#>
param(
  [int]$Cycles = 3,
  [int]$GapMs = 400,
  [string]$Root = "$env:USERPROFILE\.capy\work\kx-m2d",
  [string]$Log = ''
)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class KxSwitch {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  static void Down(IntPtr h, int vk) { PostMessage(h, 0x0100, new IntPtr(vk), new IntPtr(1)); }
  static void Up(IntPtr h, int vk) { PostMessage(h, 0x0101, new IntPtr(vk), new IntPtr(unchecked((long)0xC0000001))); }
  /// Ctrl+<digit>, down then up, as a keyboard sends it.
  public static void Ctrl(long h, int digit) {
    IntPtr w = new IntPtr(h);
    Down(w, 0x11);
    Down(w, digit);
    Up(w, digit);
    Up(w, 0x11);
  }
  public static void Show(long h, int cmd) { ShowWindow(new IntPtr(h), cmd); }
  public static void Front(long h) { SetForegroundWindow(new IntPtr(h)); }
}
"@
if (-not $Log) { $Log = Join-Path $Root 'logs\trace.err.txt' }
$hwnd = [int64](Get-Content (Join-Path $Root 'logs\hwnd.txt') -Raw)
if ($hwnd -le 0) { Write-Error "no window in $Root\logs\hwnd.txt"; exit 1 }
[KxSwitch]::Show($hwnd, 9) | Out-Null
[KxSwitch]::Front($hwnd) | Out-Null
Start-Sleep -Milliseconds 600

$start = (Select-String -Path $Log -Pattern 'input_trace' -ErrorAction SilentlyContinue | Measure-Object).Count
for ($c = 0; $c -lt $Cycles; $c++) {
  foreach ($n in 1..9) {
    # 1..9 are their own virtual-key codes, 0x31..0x39.
    [KxSwitch]::Ctrl($hwnd, 48 + $n)
    Start-Sleep -Milliseconds $GapMs
  }
}
Start-Sleep -Milliseconds 800

$lines = @(Select-String -Path $Log -Pattern 'input_trace (\d+) us' -ErrorAction SilentlyContinue |
  Select-Object -Skip $start)
$us = @($lines | ForEach-Object { [int]$_.Matches[0].Groups[1].Value })
if ($us.Count -eq 0) { Write-Error "no input_trace lines after the switches"; exit 1 }
$sorted = $us | Sort-Object
function Pct([double]$p) {
  if ($sorted.Count -eq 1) { return $sorted[0] }
  $i = [math]::Min($sorted.Count - 1, [math]::Ceiling($p * $sorted.Count) - 1)
  $sorted[[math]::Max(0, $i)]
}
"switches {0}, key to frame: p50 {1} ms, p95 {2} ms, max {3} ms" -f $us.Count, (Pct 0.50), (Pct 0.95), ($sorted[-1])
"  all (ms): " + (($us | ForEach-Object { [math]::Round($_ / 1000.0, 1) }) -join ' ')
