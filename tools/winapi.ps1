# Shared Win32 helpers for the XBGG test scripts.
if (-not ('XbggTest.Native' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace XbggTest {
public static class Native {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr proc, uint flags);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint mapType);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);

  // The real UI window: visible, owned by `pid`, not winit's tiny event-target helper.
  public static IntPtr FindUiWindow(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint owner; GetWindowThreadProcessId(h, out owner);
      if (owner != pid || !IsWindowVisible(h)) return true;
      var c = new System.Text.StringBuilder(256); GetClassName(h, c, 256);
      if (c.ToString() == "Winit Thread Event Target") return true;
      RECT r; GetWindowRect(h, out r);
      if (r.R - r.L < 100) return true;
      found = h; return false;
    }, IntPtr.Zero);
    return found;
  }
}
}
'@
}
Add-Type -AssemblyName System.Drawing

# XBGG is single-instance: a stray copy would make the one under test exit.
if (Get-Process xbgg -ErrorAction SilentlyContinue) {
    throw "xbgg.exe is already running (pid $((Get-Process xbgg).Id -join ', ')); close it before testing"
}

# Tests that launch something must not freeze the user's real apps: back up
# library.json and switch Boost off. Returns the backup path for Restore-XbggConfig.
function Disable-XbggBoost {
    $cfg = Join-Path $env:APPDATA 'XBGG\library.json'
    $bak = "$cfg.noboost-$PID"
    if (-not (Test-Path $cfg)) { return $null }
    Copy-Item $cfg $bak -Force
    $j = Get-Content $cfg -Raw | ConvertFrom-Json
    if ($j.PSObject.Properties.Name -contains 'boost') { $j.boost.enabled = $false }
    else { $j | Add-Member -NotePropertyName boost -NotePropertyValue ([pscustomobject]@{ enabled = $false }) }
    [IO.File]::WriteAllText($cfg, ($j | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding $false))
    $bak
}
function Restore-XbggConfig($bak) {
    if ($bak -and (Test-Path $bak)) { Move-Item $bak (Join-Path $env:APPDATA 'XBGG\library.json') -Force }
}

$script:VK = @{ Left = 0x25; Right = 0x27; Up = 0x26; Down = 0x28; Enter = 0x0D; Esc = 0x1B; Del = 0x2E; Ins = 0x2D; F5 = 0x74; F2 = 0x71; End = 0x23; Home = 0x24; U = 0x55 }

function Send-Key([IntPtr]$hwnd, [string]$name, [int]$times = 1, [int]$delayMs = 60) {
    $vk = $script:VK[$name]
    # Real keyboards send a scan code; winit derives letters from it.
    $scan = [int64][XbggTest.Native]::MapVirtualKey([uint32]$vk, 0)
    $ext = if ($vk -in 0x21..0x2E) { 1 -shl 24 } else { 0 }   # navigation keys are "extended"
    $down = [IntPtr](1 -bor ($scan -shl 16) -bor $ext)
    $up = [IntPtr]([int64]0xC0000001 -bor ($scan -shl 16) -bor $ext)
    for ($i = 0; $i -lt $times; $i++) {
        [XbggTest.Native]::PostMessage($hwnd, 0x100, [IntPtr]$vk, $down) | Out-Null
        [XbggTest.Native]::PostMessage($hwnd, 0x101, [IntPtr]$vk, $up) | Out-Null
        Start-Sleep -Milliseconds $delayMs
    }
}

function Save-WindowShot([IntPtr]$hwnd, [string]$path) {
    $r = New-Object XbggTest.Native+RECT
    [XbggTest.Native]::GetWindowRect($hwnd, [ref]$r) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
    [XbggTest.Native]::PrintWindow($hwnd, $hdc, 2) | Out-Null
    $g.ReleaseHdc($hdc); $g.Dispose(); $bmp.Save($path); $bmp.Dispose()
}

function Get-ForegroundPid {
    $pid2 = 0
    [XbggTest.Native]::GetWindowThreadProcessId([XbggTest.Native]::GetForegroundWindow(), [ref]$pid2) | Out-Null
    $pid2
}

function Wait-MainWindow([System.Diagnostics.Process]$p, [int]$timeoutSec = 20) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        $p.Refresh()
        if ($p.HasExited) { throw "process exited early with code $($p.ExitCode)" }
        $h = [XbggTest.Native]::FindUiWindow([uint32]$p.Id)
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 200
    }
    throw "no main window after $timeoutSec s"
}

# Memory/CPU/GPU snapshot of a process.
function Get-ProcStats([System.Diagnostics.Process]$p) {
    $p.Refresh()
    $gpuDed = 0; $gpuShared = 0
    try {
        $c = Get-Counter -Counter "\GPU Process Memory(pid_$($p.Id)_*)\Dedicated Usage", "\GPU Process Memory(pid_$($p.Id)_*)\Shared Usage" -ErrorAction Stop
        foreach ($s in $c.CounterSamples) {
            if ($s.Path -like '*dedicated usage') { $gpuDed += $s.CookedValue } else { $gpuShared += $s.CookedValue }
        }
    } catch {}
    [pscustomobject]@{
        WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        GpuDedicatedMB = [math]::Round($gpuDed / 1MB, 1)
        GpuSharedMB  = [math]::Round($gpuShared / 1MB, 1)
        Handles      = $p.HandleCount
        Threads      = $p.Threads.Count
        GdiObjects   = [XbggTest.Native]::GetGuiResources($p.Handle, 0)
        UserObjects  = [XbggTest.Native]::GetGuiResources($p.Handle, 1)
        CpuSec       = [math]::Round($p.TotalProcessorTime.TotalSeconds, 2)
    }
}

function Get-IdleCpuMs([System.Diagnostics.Process]$p, [int]$seconds = 10) {
    $p.Refresh(); $a = $p.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Seconds $seconds
    $p.Refresh(); [math]::Round($p.TotalProcessorTime.TotalMilliseconds - $a)
}
