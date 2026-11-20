<#
.SYNOPSIS
  End-to-end test of shell mode: "hide desktop while open" and "start with
  Windows". Closes and restores the real Windows taskbar several times.
  Open File Explorer windows are recorded and reopened at the end; the
  user's library.json is backed up and restored; the taskbar is always
  brought back, whatever happens.
#>
param([string]$Exe = "")
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
$Exe = (Resolve-Path $Exe).Path
. "$PSScriptRoot\winapi.ps1"
Add-Type -Namespace ShellTest -Name U -MemberDefinition @'
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
[DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr h);
[DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
'@
function Log($m) { "[{0:HH:mm:ss}] {1}" -f (Get-Date), $m }
$script:results = @()
function Check($name, [bool]$ok, $detail = '') {
    $script:results += [pscustomobject]@{ Check = $name; Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Detail = $detail }
    Log ("{0} {1} {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail)
}
function Taskbar { [ShellTest.U]::FindWindow('Shell_TrayWnd', $null) -ne [IntPtr]::Zero }
# A usable desktop: responsive taskbar and a registered shell window.
function Healthy { $t = [ShellTest.U]::FindWindow('Shell_TrayWnd', $null); $t -ne [IntPtr]::Zero -and -not [ShellTest.U]::IsHungAppWindow($t) -and [ShellTest.U]::GetShellWindow() -ne [IntPtr]::Zero }
function Wait-Healthy([int]$sec = 15) { $d = (Get-Date).AddSeconds($sec); while ((Get-Date) -lt $d) { if (Healthy) { return $true }; Start-Sleep -Milliseconds 250 }; return $false }
function Wait-Taskbar([bool]$present, [int]$sec = 10) {
    $d = (Get-Date).AddSeconds($sec)
    while ((Get-Date) -lt $d) { if ((Taskbar) -eq $present) { return $true }; Start-Sleep -Milliseconds 200 }
    return $false
}
function ShellPid { $o = 0; [ShellTest.U]::GetWindowThreadProcessId([ShellTest.U]::FindWindow('Shell_TrayWnd', $null), [ref]$o) | Out-Null; $o }
function Avail-MB { [math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1KB) }
$cfgPath = Join-Path $env:APPDATA 'XBGG\library.json'
$statePath = Join-Path $env:APPDATA 'XBGG\shell-state.json'
function Set-Hide([bool]$on) {
    $j = Get-Content $cfgPath -Raw | ConvertFrom-Json
    if ($j.PSObject.Properties.Name -notcontains 'ui') { $j | Add-Member -NotePropertyName ui -NotePropertyValue ([pscustomobject]@{}) }
    if ($j.ui.PSObject.Properties.Name -contains 'hide_desktop') { $j.ui.hide_desktop = $on } else { $j.ui | Add-Member -NotePropertyName hide_desktop -NotePropertyValue $on }
    [IO.File]::WriteAllText($cfgPath, ($j | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding $false))
}

if (-not (Taskbar)) { throw 'the taskbar is not running; start explorer.exe first' }
$folders = @((New-Object -ComObject Shell.Application).Windows() | Where-Object { $_.FullName -like '*explorer.exe' } | ForEach-Object { $_.Document.Folder.Self.Path })
Log "open folder windows to restore later: $($folders -join '; ')"
$autostartBefore = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).XBGG
$noBoost = Disable-XbggBoost      # backs up library.json, boost off for the test

try {
    # --- 0. File Explorer windows inside the shell process: must NOT close it
    $shellPid = ShellPid
    $inShell = @((New-Object -ComObject Shell.Application).Windows() | Where-Object { $_.FullName -like '*explorer.exe' } | Where-Object { $o = 0; [ShellTest.U]::GetWindowThreadProcessId([IntPtr]$_.HWND, [ref]$o) | Out-Null; $o -eq $shellPid }).Count
    Log "folder windows inside the shell process ($shellPid): $inShell"
    if ($inShell -gt 0) {
        Set-Hide $true
        $x = Start-Process $Exe -ArgumentList '--no-update' -PassThru
        $null = Wait-MainWindow $x
        Start-Sleep -Seconds 4
        Check 'folder windows open: desktop left alone' ((Healthy) -and -not (Test-Path $statePath))
        Stop-Process -Id $x.Id; Start-Sleep -Seconds 1
        # Close them (reopened at the end) to test the real flow.
        (New-Object -ComObject Shell.Application).Windows() | Where-Object { $_.FullName -like '*explorer.exe' } | ForEach-Object { $_.Quit() }
        Start-Sleep -Seconds 2
    }

    # --- 1. hide on start, restore on exit
    Set-Hide $true
    $shellBefore = ShellPid
    $before = Avail-MB
    $x = Start-Process $Exe -ArgumentList '--no-update' -PassThru
    $h = Wait-MainWindow $x
    Check 'taskbar closed when XBGG starts' (Wait-Taskbar $false 10)
    Start-Sleep -Seconds 2
    $d = (Get-Date).AddSeconds(10); while ((Get-Date) -lt $d -and (Get-Process -Id $shellBefore -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 250 }
    Check 'the shell process exited completely' (-not (Get-Process -Id $shellBefore -ErrorAction SilentlyContinue))
    $st = Get-Content $statePath -Raw -ErrorAction SilentlyContinue | ConvertFrom-Json
    Check 'shell state recorded before hiding' ($st -and $st.desktop_hidden -and $st.owner_pid -eq $x.Id)
    $guards = @(Get-CimInstance Win32_Process -Filter "Name='xbgg.exe'" | Where-Object { $_.CommandLine -match '--boost-guard' })
    Check 'guard process running' ($guards.Count -eq 1)
    Check 'XBGG in the foreground' ((Get-ForegroundPid) -eq $x.Id)
    Log "available RAM: $before MB -> $(Avail-MB) MB with the desktop closed"
    Send-Key $h Esc 1 300; Send-Key $h Enter 1 200        # exit dialog -> confirm
    Check 'XBGG exits from the menu' ($x.WaitForExit(15000))
    Check 'desktop back and responsive after exiting XBGG' (Wait-Healthy 20)
    Check 'shell state removed' (-not (Test-Path $statePath))
    Start-Sleep -Seconds 2
    Check 'guard exited' (@(Get-Process xbgg -ErrorAction SilentlyContinue).Count -eq 0)

    # --- 2. crash: the guard must bring the desktop back
    $x = Start-Process $Exe -ArgumentList '--no-update' -PassThru
    $null = Wait-MainWindow $x
    Check 'crash test: taskbar closed' (Wait-Taskbar $false 10)
    Start-Sleep -Seconds 2
    Stop-Process -Id $x.Id -Force
    Log 'XBGG killed'
    Check 'crash test: guard restored a responsive desktop' (Wait-Healthy 20)
    Check 'crash test: shell state removed' (-not (Test-Path $statePath))

    # --- 3. toggles from the menu (rows: ..., services 5, [remove helper 6], autostart, desktop)
    $desktopRow = if (Test-Path "$env:ProgramFiles\XBGG\xbgg-helper.exe") { 8 } else { 7 }
    Set-Hide $false
    Start-Sleep -Seconds 2
    $x = Start-Process $Exe -ArgumentList '--no-update' -PassThru
    $h = Wait-MainWindow $x
    Start-Sleep -Seconds 3
    Check 'off: taskbar stays' (Taskbar)
    Send-Key $h F2 1 400; Send-Key $h Down $desktopRow 80; Send-Key $h Enter 1 300
    Check 'menu toggle on: taskbar closes' (Wait-Taskbar $false 10)
    Send-Key $h Enter 1 300
    Check 'menu toggle off: responsive desktop returns' (Wait-Healthy 20)
    Send-Key $h Up 1 80; Send-Key $h Enter 1 400                       # start with windows
    $run = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).XBGG
    $expected = if ($autostartBefore) { $null } else { "`"$Exe`" --autostart" }
    Check 'start with windows toggles the Run entry' ($run -eq $expected) "$run"
    Send-Key $h Enter 1 400                                           # and back
    $run = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).XBGG
    Check 'start with windows toggles back' ($run -eq $autostartBefore) "$run"
    Send-Key $h Esc 1 200
    Stop-Process -Id $x.Id
}
finally {
    Get-Process xbgg -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Restore-XbggConfig $noBoost
    Remove-Item $statePath -ErrorAction SilentlyContinue
    if (-not (Healthy)) {
        Log 'failsafe: restarting explorer'
        Get-Process explorer -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Seconds 2
        if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process "$env:WINDIR\explorer.exe" }
        $null = Wait-Healthy 30
    }
    Start-Sleep -Seconds 2
    foreach ($f in $folders) { if ($f -and (Test-Path $f)) { Start-Process "$env:WINDIR\explorer.exe" -ArgumentList "`"$f`"" } }
    Log "desktop healthy: $(Healthy); reopened $($folders.Count) folder window(s)"
}
$script:results | Format-Table -AutoSize | Out-String -Width 200
$fails = @($script:results | Where-Object Result -eq 'FAIL').Count
Log $(if ($fails -eq 0) { 'RESULT: PASS' } else { "RESULT: FAIL ($fails)" })
