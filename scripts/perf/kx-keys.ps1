<#
.SYNOPSIS
  Types into the window the way a person does — real key events through the
  OS — and leaves the measurement to the app (`ZGX_INPUT_TRACE=1` prints key
  dispatch to the frame that shows it).

.DESCRIPTION
  `zgx inspect type` writes text without a key event and `zgx inspect key`
  dispatches one that inserts nothing, so neither measures typing. This posts
  real WM_KEYDOWN/WM_CHAR pairs with `SendInput` into the focused window,
  which is the path a keyboard takes.

  The numbers are the app's own: `input_trace <us> us (key …, frame …)` line
  per key that reached the screen.
#>
param(
  [int]$Keys = 40,
  [int]$GapMs = 160,
  [string]$Text = 'abcdefghijklmnopqrstuvwxyz0123456789abcdefghij',
  [string]$Root = "$env:USERPROFILE\.capy\work\kx-perf",
  [string]$Log = ''
)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class KxKeys {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  public delegate bool Proc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  /// The largest visible window of `pid` whose title holds nothing but the
  /// app's name: the launcher's says "Keplorayx launcher" and lives offscreen.
  public static IntPtr FindMain(uint pid) {
    IntPtr found = IntPtr.Zero; int biggest = 0;
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      if (!IsWindowVisible(h)) return true;
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
      var sb = new StringBuilder(512); GetWindowTextW(h, sb, 512);
      var text = sb.ToString();
      if (text.IndexOf("Keplorayx", StringComparison.OrdinalIgnoreCase) < 0) return true;
      if (text.IndexOf("launcher", StringComparison.OrdinalIgnoreCase) >= 0) return true;
      RECT r; GetWindowRect(h, out r);
      int area = (r.R - r.L) * (r.B - r.T);
      if (area > biggest) { biggest = area; found = h; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public KEYBD k; public int pad1; public int pad2; }
  [StructLayout(LayoutKind.Explicit)] public struct KEYBD {
    [FieldOffset(0)] public ushort wVk;
    [FieldOffset(2)] public ushort wScan;
    [FieldOffset(4)] public uint dwFlags;
    [FieldOffset(8)] public uint time;
    [FieldOffset(12)] public IntPtr extra;
  }
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  /// A key the way a keyboard sends one: down, its character, up — posted to
  /// the window rather than typed into it, so the app takes it from its own
  /// input queue.
  public static void Post(IntPtr h, ushort vk, char c) {
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)1);
    PostMessage(h, 0x0102, (IntPtr)c, (IntPtr)1);
    PostMessage(h, 0x0101, (IntPtr)vk, (IntPtr)0xC0000001);
  }
  public static void Key(ushort vk, ushort scan) {
    var down = new INPUT { type = 1, k = new KEYBD { wVk = vk, wScan = scan, dwFlags = 0 } };
    var up = new INPUT { type = 1, k = new KEYBD { wVk = vk, wScan = scan, dwFlags = 2 } };
    SendInput(1, new[] { down }, Marshal.SizeOf(typeof(INPUT)));
    SendInput(1, new[] { up }, Marshal.SizeOf(typeof(INPUT)));
  }
}
"@
if (-not $Log) { $Log = Join-Path $Root 'logs\trace.err.txt' }
$pid_ = [int](Get-Content (Join-Path $Root 'logs\pid.txt') -Raw)
$p = Get-Process -Id $pid_ -ErrorAction Stop
$h = [KxKeys]::FindMain([uint32]$pid_)
if ($h -eq [IntPtr]::Zero) { Write-Error "no main window for pid $pid_"; exit 1 }
[KxKeys]::ShowWindow($h, 9) | Out-Null
[KxKeys]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500
$before = (Select-String -Path $Log -Pattern 'input_trace' -ErrorAction SilentlyContinue | Measure-Object).Count
for ($i = 0; $i -lt $Keys; $i++) {
  $c = $Text[$i % $Text.Length]
  # Letters and digits have the same virtual-key code as their character.
  $vk = [int][char]([string]$c).ToUpper()
  [KxKeys]::Post($h, [uint16]$vk, [char]$c)
  Start-Sleep -Milliseconds $GapMs
}
Start-Sleep -Milliseconds 400
$after = (Select-String -Path $Log -Pattern 'input_trace' -ErrorAction SilentlyContinue | Measure-Object).Count
"typed $Keys keys into pid $pid_, traces $before -> $after"
