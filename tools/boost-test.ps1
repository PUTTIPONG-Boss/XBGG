<#
.SYNOPSIS
  End-to-end Boost test with a real game.

  - A test program ("memhog", holds -MemoryMB of RAM) and Wallpaper Engine
    (if running) are the apps to freeze; nothing else of the user's is touched.
  - Launches -GameTitle from XBGG, checks that the apps are frozen, RAM was
    released, the game runs at High priority, the journal and guard exist;
    closes the game and checks everything is resumed and cleaned up.
  - Crash test: starts a boost on winver.exe, kills XBGG, and checks that the
    guard process restores everything.
  The user's library.json is backed up and restored.
#>
param(
    [string]$Exe = "",
    [string]$GameProcess = "deadlock",
    [string]$GameTitle = "Deadlock",
    [int]$MemoryMB = 1500,
    [int]$PlaySeconds = 30,
    # Also pause Windows services (needs the admin helper installed).
    [switch]$Services,
    [string]$ShotDir = ""
)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
$Exe = (Resolve-Path $Exe).Path
. "$PSScriptRoot\winapi.ps1"
function Log($m) { "[{0:HH:mm:ss}] {1}" -f (Get-Date), $m }
function Check($name, [bool]$ok, $detail = '') {
    $script:results += [pscustomobject]@{ Check = $name; Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Detail = $detail }
    Log ("{0} {1} {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail)
}
# Frozen = nothing running and (nearly) every thread suspended. A thread that
# is inside a kernel wait when the process is suspended shows "Executive"
# until that wait ends; the suspension applies the moment it returns.
function Is-Frozen([int]$procId) {
    $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
    if (-not $p) { return $false }
    $t = @($p.Threads)
    $susp = @($t | Where-Object { $_.ThreadState -eq 'Wait' -and $_.WaitReason -eq 'Suspended' }).Count
    $running = @($t | Where-Object { $_.ThreadState -in 'Running', 'Ready' }).Count
    $t.Count -gt 0 -and $running -eq 0 -and $susp -ge [math]::Ceiling($t.Count * 0.9)
}
$svcNames = 'WSearch', 'SysMain', 'DiagTrack'
function Svc-State { ($svcNames | ForEach-Object { $s = Get-Service $_; "$($s.Name)=$($s.Status)/$($s.StartType)" }) -join ', ' }
function Svc-AllStopped { @($svcNames | Where-Object { (Get-Service $_).Status -ne 'Stopped' }).Count -eq 0 }
function Wait-Svc([string]$want, [int]$sec = 30) { $d = (Get-Date).AddSeconds($sec); while ((Get-Date) -lt $d) { if (@($svcNames | Where-Object { (Get-Service $_).Status -ne $want }).Count -eq 0) { return $true }; Start-Sleep -Milliseconds 500 }; $false }
function Avail-MB { [math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1KB) }
function Get-XbggList {
    $tmp = [IO.Path]::GetTempFileName()
    Start-Process $Exe -ArgumentList '--list' -Wait -NoNewWindow -RedirectStandardOutput $tmp | Out-Null
    Get-Content $tmp | Where-Object { $_ -match '^\S' -and $_ -notmatch ' entries$' }
    Remove-Item $tmp
}
function Index-Of($title) {
    $l = @(Get-XbggList)
    for ($i = 0; $i -lt $l.Count; $i++) { if ($l[$i] -match "^\S+(\s\S+)?\s+$([regex]::Escape($title))\s") { return $i } }
    throw "'$title' not in library"
}

$script:results = @()
$work = Join-Path ([IO.Path]::GetTempPath()) "xbgg-boost-test-$PID"
New-Item -ItemType Directory $work -Force | Out-Null
$config = Join-Path $env:APPDATA 'XBGG\library.json'
$journal = Join-Path $env:APPDATA 'XBGG\boost-journal.json'
$backup = "$config.boosttest"
Copy-Item $config $backup -Force

# --- test program that holds memory
$memhog = Join-Path $work 'memhog.exe'
Add-Type -OutputType ConsoleApplication -OutputAssembly $memhog -TypeDefinition @'
using System; using System.Collections.Generic; using System.Threading;
public static class MemHog { public static void Main(string[] a) {
  int mb = int.Parse(a[0]); var keep = new List<byte[]>();
  for (int i = 0; i < mb; i++) { var b = new byte[1 << 20]; for (int j = 0; j < b.Length; j += 4096) b[j] = 1; keep.Add(b); }
  Console.WriteLine("ready");
  // Like a real app, keep using the memory (pulls it back in after a resume).
  while (true) { foreach (var b in keep) for (int j = 0; j < b.Length; j += 4096) b[j]++; Thread.Sleep(1000); } } }
'@

$hog = $null; $x = $null; $game = $null; $winver = $null
try {
    # --- configure Boost for the test
    $cfg = Get-Content $config -Raw | ConvertFrom-Json
    $boost = [pscustomobject]@{ enabled = $true; freeze_apps = $true; freeze_list = @('memhog.exe', 'wallpaper32.exe'); power_mode = $true; game_priority = $true; pause_services = [bool]$Services }
    $cfg | Add-Member -NotePropertyName boost -NotePropertyValue $boost -Force
    $cfg.custom = @([pscustomobject]@{ id = 'custom:boosttest'; title = 'zz Boost Test Winver'; path = "$env:WINDIR\System32\winver.exe"; args = @() })
    [IO.File]::WriteAllText($config, ($cfg | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding $false))

    $hog = Start-Process $memhog -ArgumentList $MemoryMB -PassThru -WindowStyle Hidden
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Process -Id $hog.Id).WorkingSet64 -lt ($MemoryMB * 0.9 * 1MB) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
    Start-Sleep -Seconds 2
    $hogWsBefore = [math]::Round((Get-Process -Id $hog.Id).WorkingSet64 / 1MB)
    $wp = Get-Process wallpaper32 -ErrorAction SilentlyContinue | Select-Object -First 1
    $svcBefore = Svc-State
    if ($Services) { Log "services before: $svcBefore" }
    $availBefore = Avail-MB
    Log "memhog holds $hogWsBefore MB; available RAM $availBefore MB; wallpaper32 running: $([bool]$wp)"

    # --- session 1: real game
    $gameIndex = Index-Of $GameTitle
    $x = Start-Process $Exe -ArgumentList '--no-update' -PassThru
    $h = Wait-MainWindow $x
    Start-Sleep -Seconds 3
    Send-Key $h Home 1 100
    if ($gameIndex -gt 0) { Send-Key $h Right $gameIndex 120 }
    Start-Sleep -Milliseconds 600
    Send-Key $h Enter 1 100
    $deadline = (Get-Date).AddSeconds(120)
    while (-not $game -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500; $game = Get-Process $GameProcess -ErrorAction SilentlyContinue | Select-Object -First 1 }
    if (-not $game) { throw "$GameProcess did not start" }
    Log "$GameProcess started (pid $($game.Id))"
    Start-Sleep -Seconds 8

    Check 'memhog frozen' (Is-Frozen $hog.Id)
    $hogWsDuring = [math]::Round((Get-Process -Id $hog.Id).WorkingSet64 / 1MB)
    Check 'memhog RAM released' ($hogWsDuring -lt $hogWsBefore * 0.2) "$hogWsBefore MB -> $hogWsDuring MB"
    if ($wp) { Check 'wallpaper32 frozen' (Is-Frozen $wp.Id) }
    $all = @(Get-Process $GameProcess -ErrorAction SilentlyContinue)
    $prio = ($all | ForEach-Object { "pid $($_.Id) started $($_.StartTime.ToString('HH:mm:ss')): $($_.PriorityClass)" }) -join '; '
    Check 'game at High priority' (@($all | Where-Object { $_.PriorityClass -ne 'High' }).Count -eq 0) $prio
    Check 'game itself not frozen' (-not (Is-Frozen $game.Id))
    if ($Services) {
        $ok = Wait-Svc 'Stopped' 20
        Check 'services paused (stopped + disabled) while playing' ($ok -and @($svcNames | Where-Object { (Get-Service $_).StartType -ne 'Disabled' }).Count -eq 0) (Svc-State)
    }
    Check 'journal written' (Test-Path $journal)
    $guards = @(Get-CimInstance Win32_Process -Filter "Name='xbgg.exe'" | Where-Object { $_.CommandLine -match '--boost-guard' })
    Check 'guard process running' ($guards.Count -eq 1) "$($guards.Count) guard(s)"

    $g0 = if ($guards) { (Get-Process -Id $guards[0].ProcessId).TotalProcessorTime.TotalMilliseconds } else { 0 }
    $threads = @(& "$PSScriptRoot\threads.ps1" -ProcessId $x.Id -Seconds $PlaySeconds)
    $cpu = ($threads | Measure-Object CpuMs -Sum).Sum
    $gcpu = if ($guards) { [math]::Round((Get-Process -Id $guards[0].ProcessId).TotalProcessorTime.TotalMilliseconds - $g0) } else { 0 }
    $top = ($threads | Where-Object CpuMs -gt 0 | Select-Object -First 3 | ForEach-Object { "$($_.Name)[$($_.Tid)] $($_.CpuMs)ms" }) -join ', '
    Check 'XBGG CPU while playing stays ~0' ($cpu -le 100) "$cpu ms XBGG + $gcpu ms guard over $PlaySeconds s ($top)"
    $all = @(Get-Process $GameProcess -ErrorAction SilentlyContinue)
    Check 'game still at High priority later' (@($all | Where-Object { $_.PriorityClass -ne 'High' }).Count -eq 0) (($all | ForEach-Object { "pid $($_.Id): $($_.PriorityClass)" }) -join '; ')
    $game = $all | Select-Object -First 1

    $game.CloseMainWindow() | Out-Null
    if (-not $game.WaitForExit(60000)) { throw 'game did not close' }
    $closedAt = Get-Date
    Log 'game closed'
    $deadline = (Get-Date).AddSeconds(30)
    while ((Test-Path $journal) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
    $restoreSec = [math]::Round(((Get-Date) - $closedAt).TotalSeconds, 1)
    Start-Sleep -Seconds 1
    Check 'journal removed after game exit' (-not (Test-Path $journal))
    Check 'apps restored quickly after the game closed' ($restoreSec -le 8) "$restoreSec s"
    if ($Services) {
        $null = Wait-Svc 'Running' 30
        Check 'services resumed with their original start types' ((Svc-State) -eq $svcBefore) (Svc-State)
    }
    Check 'memhog resumed' (-not (Is-Frozen $hog.Id))
    if ($wp) { Check 'wallpaper32 resumed' (-not (Is-Frozen $wp.Id)) }
    if ($ShotDir) { Start-Sleep -Seconds 2; Save-WindowShot $h (Join-Path $ShotDir 'boost-after-game.png') }

    # --- session 2: crash test with a directly-started program (PID tracked)
    $wIndex = Index-Of 'zz Boost Test Winver'
    Send-Key $h Home 1 100
    if ($wIndex -gt 0) { Send-Key $h Right $wIndex 120 }
    Start-Sleep -Seconds 4   # launch cooldown
    # Nothing else is loading here, so the RAM gain is measurable cleanly.
    $availBefore = Avail-MB
    Send-Key $h Enter 1 100
    $deadline = (Get-Date).AddSeconds(20)
    while (-not $winver -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300; $winver = Get-Process winver -ErrorAction SilentlyContinue | Select-Object -First 1 }
    if (-not $winver) { throw 'winver did not start' }
    Start-Sleep -Seconds 4
    Check 'crash test: memhog frozen before crash' (Is-Frozen $hog.Id)
    $availAfter = Avail-MB
    Check 'available RAM increased by the frozen app' (($availAfter - $availBefore) -ge ($MemoryMB * 0.8)) "$availBefore MB -> $availAfter MB (+$($availAfter - $availBefore) MB)"
    Stop-Process -Id $x.Id -Force       # simulate XBGG crashing
    Log 'XBGG killed'
    $deadline = (Get-Date).AddSeconds(15)
    while ((Test-Path $journal) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
    Start-Sleep -Seconds 1
    Check 'crash test: guard restored memhog' (-not (Is-Frozen $hog.Id))
    if ($Services) {
        $null = Wait-Svc 'Running' 30
        Check 'crash test: guard resumed the services' ((Svc-State) -eq $svcBefore) (Svc-State)
    }
    if ($wp) { Check 'crash test: guard restored wallpaper32' (-not (Is-Frozen $wp.Id)) }
    Check 'crash test: journal removed' (-not (Test-Path $journal))
    Start-Sleep -Seconds 2
    $left = @(Get-CimInstance Win32_Process -Filter "Name='xbgg.exe'")
    Check 'crash test: guard exited' ($left.Count -eq 0) "$($left.Count) xbgg process(es) left"
}
finally {
    foreach ($p in @($winver, $game)) { if ($p -and -not $p.HasExited) { $p.CloseMainWindow() | Out-Null; Start-Sleep 1; if (-not $p.HasExited) { Stop-Process -Id $p.Id -ErrorAction SilentlyContinue } } }
    Get-Process xbgg -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    # Never leave anything frozen, whatever happened above.
    if (Test-Path $journal) { Start-Process $Exe -ArgumentList '--restore' -Wait -NoNewWindow -RedirectStandardOutput "$work\x.txt" | Out-Null; Log "cleanup: $(Get-Content "$work\x.txt")" }
    if ($hog -and -not $hog.HasExited) { Stop-Process -Id $hog.Id -Force -ErrorAction SilentlyContinue }
    Move-Item $backup $config -Force
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    Log 'library.json restored, test processes stopped'
}
$script:results | Format-Table -AutoSize | Out-String -Width 200
$fails = @($script:results | Where-Object Result -eq 'FAIL').Count
Log $(if ($fails -eq 0) { 'RESULT: PASS' } else { "RESULT: FAIL ($fails)" })
