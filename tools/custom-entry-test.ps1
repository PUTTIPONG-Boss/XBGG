<#
.SYNOPSIS
  End-to-end test of "Add game or app": drives the real file picker, launches
  the added entry, then removes it. The user's library.json is backed up and
  restored afterwards.
#>
param([string]$Exe = "", [string]$Program = "$env:WINDIR\System32\winver.exe", [string]$ShotDir = "")
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
. "$PSScriptRoot\winapi.ps1"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -Namespace XbggDlg -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dlg, int id);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string l);
[DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
'@
# Common file dialog: the "File name" box is control 1148 (ComboBoxEx32 > ComboBox > Edit).
function Set-DialogFileName([IntPtr]$dlg, [string]$path) {
    $cbx = [XbggDlg.U]::GetDlgItem($dlg, 1148)
    if ($cbx -eq [IntPtr]::Zero) { throw 'file name box not found' }
    $edit = [XbggDlg.U]::FindWindowEx($cbx, [IntPtr]::Zero, 'ComboBox', $null)
    if ($edit -ne [IntPtr]::Zero) { $edit = [XbggDlg.U]::FindWindowEx($edit, [IntPtr]::Zero, 'Edit', $null) }
    if ($edit -eq [IntPtr]::Zero) { $edit = $cbx }
    [XbggDlg.U]::SendMessage($edit, 0x000C, [IntPtr]::Zero, $path) | Out-Null     # WM_SETTEXT
    Start-Sleep -Milliseconds 300
    [XbggDlg.U]::SendMessage($dlg, 0x0111, [IntPtr]1, [IntPtr]::Zero) | Out-Null  # WM_COMMAND IDOK
}
function Log($m) { "[{0:HH:mm:ss}] {1}" -f (Get-Date), $m }
function Find-Dialog([uint32]$procId) {
    $found = [IntPtr]::Zero
    $cb = [XbggTest.Native+EnumProc] {
        param($h, $l)
        $o = 0; [XbggTest.Native]::GetWindowThreadProcessId($h, [ref]$o) | Out-Null
        $c = New-Object System.Text.StringBuilder 64; [XbggTest.Native]::GetClassName($h, $c, 64) | Out-Null
        if ($o -eq $procId -and $c.ToString() -eq '#32770' -and [XbggTest.Native]::IsWindowVisible($h)) { $script:dlg = $h; return $false }
        return $true
    }
    $script:dlg = [IntPtr]::Zero
    [XbggTest.Native]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
    $script:dlg
}

$config = Join-Path $env:APPDATA 'XBGG\library.json'
$backup = "$config.testbackup"
if (Test-Path $config) { Copy-Item $config $backup -Force }
$name = [IO.Path]::GetFileNameWithoutExtension($Program)
$result = 'FAIL'
try {
    $x = Start-Process (Resolve-Path $Exe) -ArgumentList '--no-update' -PassThru
    $h = Wait-MainWindow $x
    Start-Sleep -Seconds 2
    Send-Key $h Ins 1 100
    $deadline = (Get-Date).AddSeconds(10); $d = [IntPtr]::Zero
    while ($d -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300; $d = Find-Dialog ([uint32]$x.Id) }
    if ($d -eq [IntPtr]::Zero) { throw 'file picker did not open' }
    Log 'file picker opened'
    # While the picker is open, XBGG input must be blocked: Enter must not launch.
    Send-Key $h Enter 1 100
    Send-Key $h Right 1 100
    Start-Sleep -Milliseconds 500
    if (Get-Process $name -ErrorAction SilentlyContinue) { throw 'input reached XBGG while the picker was open' }
    Set-DialogFileName $d $Program
    Start-Sleep -Seconds 2
    if ((Find-Dialog ([uint32]$x.Id)) -ne [IntPtr]::Zero) { throw 'file picker still open' }

    $json = Get-Content $config -Raw | ConvertFrom-Json
    $entry = $json.custom | Where-Object { $_.path -eq $Program }
    if (-not $entry) { throw "entry for $Program not saved to library.json" }
    Log "saved to library.json as $($entry.id) ('$($entry.title)')"
    if ($ShotDir) { Save-WindowShot $h (Join-Path $ShotDir 'custom-1-added.png') }

    # The new entry is focused: Enter launches it.
    Log "foreground after picker closed is XBGG: $((Get-ForegroundPid) -eq $x.Id)"
    Send-Key $h Enter 1 100
    $deadline = (Get-Date).AddSeconds(10); $proc = $null
    while (-not $proc -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300; $proc = Get-Process $name -ErrorAction SilentlyContinue }
    if ($ShotDir) { Save-WindowShot $h (Join-Path $ShotDir 'custom-1b-after-enter.png') }
    if (-not $proc) { throw "$name did not start" }
    Log "$name started (pid $($proc.Id)); launched once: $(@($proc).Count -eq 1)"
    $proc | ForEach-Object { $_.CloseMainWindow() | Out-Null }
    Start-Sleep -Seconds 2
    $proc | Where-Object { -not $_.HasExited } | Stop-Process

    # Remove it again: Delete opens the confirmation, Enter confirms.
    [XbggTest.Native]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [XbggTest.Native]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
    [XbggTest.Native]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Seconds 4   # launch cooldown + re-sort
    Send-Key $h Home 1 100
    $json = Get-Content $config -Raw | ConvertFrom-Json
    # Recently played first: the entry we just launched is now first.
    Send-Key $h Del 1 200
    if ($ShotDir) { Save-WindowShot $h (Join-Path $ShotDir 'custom-2-remove-dialog.png') }
    Send-Key $h Enter 1 300
    Start-Sleep -Seconds 1
    $json = Get-Content $config -Raw | ConvertFrom-Json
    if ($json.custom | Where-Object { $_.path -eq $Program }) { throw 'entry still present after remove' }
    Log 'entry removed from library.json'
    $x.Refresh(); if ($x.HasExited) { throw 'XBGG exited unexpectedly' }
    $result = 'PASS'
}
finally {
    if ($x -and -not $x.HasExited) { Stop-Process -Id $x.Id }
    if (Test-Path $backup) { Move-Item $backup $config -Force } elseif (Test-Path $config) { Remove-Item $config }
    Log "library.json restored"
}
Log "RESULT: $result"
