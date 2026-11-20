<#
.SYNOPSIS
  Repeats one kind of action many times and samples private memory, to find
  which action (if any) grows memory. Modes: nav, menu, rescan.
#>
param([ValidateSet('nav', 'menu', 'rescan', 'layout')] [string]$Mode = 'nav', [int]$Rounds = 400, [string]$Exe = "")
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
. "$PSScriptRoot\winapi.ps1"

$p = Start-Process (Resolve-Path $Exe) -ArgumentList '--no-update' -PassThru
$h = Wait-MainWindow $p
Start-Sleep -Seconds 3
$layoutRow = if (Test-Path "$env:ProgramFiles\XBGG\xbgg-helper.exe") { 9 } else { 8 }
$samples = @()
for ($i = 1; $i -le $Rounds; $i++) {
    switch ($Mode) {
        'nav' { Send-Key $h Down 1 90; Send-Key $h Up 1 90 }        # two cover loads
        'menu' { Send-Key $h F2 1 50; Send-Key $h Down 2 30; Send-Key $h Enter 1 120; Send-Key $h Esc 2 60 }
        'rescan' { Send-Key $h F5 1 150 }
        # Menu rows: ... services 5, [remove helper 6], autostart, desktop, layout.
        'layout' { Send-Key $h F2 1 80; Send-Key $h Down $layoutRow 30; Send-Key $h Enter 1 600; Send-Key $h Esc 1 300 }
    }
    if ($i % [math]::Max(1, [int]($Rounds / 10)) -eq 0) {
        Start-Sleep -Milliseconds 400
        $p.Refresh()
        $samples += [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
    }
}
Stop-Process -Id $p.Id
"$Mode x$Rounds private MB: $($samples -join ' ')"
