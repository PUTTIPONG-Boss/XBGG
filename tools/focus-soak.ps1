<#
.SYNOPSIS
  Switches the foreground between XBGG and a stand-in game many times and
  checks that threads/handles do not grow (regression test for leaked
  controller-polling threads) and that background CPU stays near zero.
#>
param([string]$Exe = "", [int]$Switches = 40)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
. "$PSScriptRoot\winapi.ps1"

function Focus([IntPtr]$hwnd) {
    # Tapping Alt lets this script move the foreground (foreground lock rules).
    [XbggTest.Native]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
    [XbggTest.Native]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
    [XbggTest.Native]::SetForegroundWindow($hwnd) | Out-Null
}

$x = Start-Process (Resolve-Path $Exe) -ArgumentList '--no-update', '--windowed' -PassThru
$xh = Wait-MainWindow $x
$g = Start-Process powershell -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$PSScriptRoot\fake-game.ps1", '-Seconds', '600' -PassThru
$gh = Wait-MainWindow $g
Start-Sleep -Seconds 2

$start = Get-ProcStats $x
$ok = 0
for ($i = 1; $i -le $Switches; $i++) {
    Focus $xh; Start-Sleep -Milliseconds 700
    if ((Get-ForegroundPid) -eq $x.Id) { $ok++ }
    Focus $gh; Start-Sleep -Milliseconds 700
}
Start-Sleep -Seconds 2
$end = Get-ProcStats $x
$bgCpu = Get-IdleCpuMs $x 20
Stop-Process -Id $g.Id -ErrorAction SilentlyContinue
Stop-Process -Id $x.Id

[pscustomobject]@{
    Switches = $Switches
    XbggReachedForeground = "$ok/$Switches"
    ThreadsStart = $start.Threads; ThreadsEnd = $end.Threads
    HandlesStart = $start.Handles; HandlesEnd = $end.Handles
    PrivateMBStart = $start.PrivateMB; PrivateMBEnd = $end.PrivateMB
    BackgroundCpuMsPer20s = $bgCpu
}
