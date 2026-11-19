<#
.SYNOPSIS
  Leak test: drives XBGG through many navigate / dialog / rescan cycles and
  records memory, handles, GDI/USER objects and threads after each cycle.
#>
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\xbgg.exe",
    [int]$Cycles = 150,
    [string]$Csv = ""
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\winapi.ps1"

$p = Start-Process (Resolve-Path $Exe) -ArgumentList '--no-update' -PassThru
$h = Wait-MainWindow $p
Start-Sleep -Seconds 3

$rows = @()
for ($c = 1; $c -le $Cycles; $c++) {
    Send-Key $h End 1 40          # to the "Add" tile
    Send-Key $h Home 1 40         # back to the first game
    Send-Key $h Right 3 40        # hero swaps on the way
    Send-Key $h Esc 1 40          # open exit dialog
    Send-Key $h Esc 1 40          # cancel it
    Send-Key $h Del 1 40          # open remove dialog
    Send-Key $h Esc 1 40          # cancel it
    if ($c % 5 -eq 0) { Send-Key $h F5 1 150 }   # full rescan + artwork reload
    Start-Sleep -Milliseconds 250
    if ($p.HasExited) { throw "XBGG exited during cycle $c (code $($p.ExitCode))" }
    $s = Get-ProcStats $p
    $rows += [pscustomobject]@{
        Cycle = $c; PrivateMB = $s.PrivateMB; WorkingSetMB = $s.WorkingSetMB
        Handles = $s.Handles; Gdi = $s.GdiObjects; User = $s.UserObjects; Threads = $s.Threads
    }
}
Stop-Process -Id $p.Id

if ($Csv) { $rows | Export-Csv -NoTypeInformation -Path $Csv }

# Compare the second tenth (after warm-up) with the last tenth.
$n = [math]::Max(1, [int]($Cycles / 10))
$early = $rows | Select-Object -Skip $n -First $n
$late = $rows | Select-Object -Last $n
function Avg($set, $prop) { [math]::Round(($set | Measure-Object -Property $prop -Average).Average, 1) }
[pscustomobject]@{
    Cycles = $Cycles
    PrivateMB_Early = Avg $early PrivateMB; PrivateMB_Late = Avg $late PrivateMB
    PrivateMB_Max = ($rows | Measure-Object PrivateMB -Maximum).Maximum
    Handles_Early = Avg $early Handles; Handles_Late = Avg $late Handles
    Gdi_Early = Avg $early Gdi; Gdi_Late = Avg $late Gdi
    User_Early = Avg $early User; User_Late = Avg $late User
    Threads_Early = Avg $early Threads; Threads_Late = Avg $late Threads
}
