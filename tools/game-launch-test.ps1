<#
.SYNOPSIS
  End-to-end test with a real game: launch it from XBGG, measure XBGG while
  the game runs, close the game with Alt+F4 and check XBGG comes back.
.EXAMPLE
  tools/game-launch-test.ps1 -GameProcess deadlock -Title Deadlock
#>
param(
    [string]$Exe = "",
    [Parameter(Mandatory)] [string]$GameProcess,   # e.g. "deadlock"
    [Parameter(Mandatory)] [string]$Title,         # entry title in XBGG
    [int]$StartTimeoutSec = 180,
    [int]$PlaySeconds = 45,
    [string]$ShotDir = ""
)
$ErrorActionPreference = 'Stop'
# ($PSScriptRoot is empty in param defaults of advanced scripts on Windows PowerShell 5.1.)
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
. "$PSScriptRoot\winapi.ps1"
function Log($m) { "[{0:HH:mm:ss}] {1}" -f (Get-Date), $m }

if (Get-Process $GameProcess -ErrorAction SilentlyContinue) { throw "$GameProcess is already running" }

# xbgg.exe is a GUI-subsystem program: PowerShell's `&` does not wait for it.
function Get-XbggList {
    $tmp = [IO.Path]::GetTempFileName()
    Start-Process (Resolve-Path $Exe) -ArgumentList '--list' -Wait -NoNewWindow -RedirectStandardOutput $tmp | Out-Null
    Get-Content $tmp | Where-Object { $_ -match '^\S' -and $_ -notmatch ' entries$' }
    Remove-Item $tmp
}

# Find the entry's position in XBGG's (sorted) list.
$list = @(Get-XbggList)
$index = -1
for ($i = 0; $i -lt $list.Count; $i++) { if ($list[$i] -match "^\S+\s+$([regex]::Escape($Title))\s") { $index = $i; break } }
if ($index -lt 0) { throw "'$Title' not found in XBGG library" }
Log "'$Title' is entry #$index"

$x = Start-Process (Resolve-Path $Exe) -PassThru
$h = Wait-MainWindow $x
Start-Sleep -Seconds 3
Send-Key $h Home 1 100
if ($index -gt 0) { Send-Key $h Right $index 120 }
Start-Sleep -Milliseconds 800
if ($ShotDir) { Save-WindowShot $h (Join-Path $ShotDir 'launch-1-selected.png') }
$before = Get-ProcStats $x
Log ("XBGG before launch: private {0} MB, threads {1}" -f $before.PrivateMB, $before.Threads)

$sw = [Diagnostics.Stopwatch]::StartNew()
Send-Key $h Enter 1 100
Send-Key $h Enter 1 100   # a double press must not launch twice (cooldown)
$game = $null
while ($sw.Elapsed.TotalSeconds -lt $StartTimeoutSec -and -not $game) {
    Start-Sleep -Milliseconds 500
    $game = Get-Process $GameProcess -ErrorAction SilentlyContinue | Select-Object -First 1
}
if (-not $game) { throw "$GameProcess did not start within $StartTimeoutSec s" }
$procStart = $sw.Elapsed.TotalSeconds
Log ("game process started after {0:N1} s (pid {1})" -f $procStart, $game.Id)
$count = @(Get-Process $GameProcess -ErrorAction SilentlyContinue).Count
Log "game instances running: $count"

$fgAt = $null
while ($sw.Elapsed.TotalSeconds -lt $StartTimeoutSec) {
    if ((Get-ForegroundPid) -eq $game.Id) { $fgAt = $sw.Elapsed.TotalSeconds; break }
    Start-Sleep -Milliseconds 500
}
if ($fgAt) { Log ("game window in foreground after {0:N1} s" -f $fgAt) }
else { Log "WARNING: game never took the foreground (fg pid $(Get-ForegroundPid))" }

Start-Sleep -Seconds 5
$cpu = Get-IdleCpuMs $x $PlaySeconds
$during = Get-ProcStats $x
Log ("XBGG while game runs ({0}s): CPU {1} ms total ({2:N2}% of one core), private {3} MB, threads {4}, GPU {5} MB" -f `
    $PlaySeconds, $cpu, ($cpu / ($PlaySeconds * 10)), $during.PrivateMB, $during.Threads, $during.GpuDedicatedMB)
$g = Get-ProcStats $game
Log ("game: private {0} MB, GPU dedicated {1} MB" -f $g.PrivateMB, $g.GpuDedicatedMB)

# Alt+F4 to the game window.
[XbggTest.Native]::SetForegroundWindow($game.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500
[XbggTest.Native]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)      # Alt down
[XbggTest.Native]::keybd_event(0x73, 0, 0, [UIntPtr]::Zero)      # F4 down
[XbggTest.Native]::keybd_event(0x73, 0, 2, [UIntPtr]::Zero)      # F4 up
[XbggTest.Native]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)      # Alt up
Log "sent Alt+F4"
$exitWait = [Diagnostics.Stopwatch]::StartNew()
while (-not $game.HasExited -and $exitWait.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 500; $game.Refresh() }
if (-not $game.HasExited) {
    Log "game still running after Alt+F4; asking it to close (WM_CLOSE)"
    $game.CloseMainWindow() | Out-Null
    $exitWait.Restart()
    while (-not $game.HasExited -and $exitWait.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 500; $game.Refresh() }
}
Log ("game exited: {0} after {1:N1} s" -f $game.HasExited, $exitWait.Elapsed.TotalSeconds)

Start-Sleep -Seconds 3
$back = (Get-ForegroundPid) -eq $x.Id
Log "XBGG back in foreground: $back"
if ($ShotDir) { Save-WindowShot $h (Join-Path $ShotDir 'launch-2-after.png') }
$x.Refresh()
Log "XBGG still running: $(-not $x.HasExited)"
Stop-Process -Id $x.Id

$after = Get-XbggList | Select-Object -First 1
Log "first entry after launch (recently played first): $after"
