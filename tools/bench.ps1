<#
.SYNOPSIS
  Repeatable resource benchmark for XBGG (startup, idle, navigation, memory, GPU).
.EXAMPLE
  pwsh tools/bench.ps1 -Exe target/release/xbgg.exe -Shot bench.png
#>
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\xbgg.exe",
    [string]$Shot = "",
    [switch]$Fullscreen
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\winapi.ps1"
Add-Type -Namespace XbggBench -Name U -MemberDefinition '[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);'

$argsList = @('--no-update')
if (-not $Fullscreen) { $argsList += '--windowed' }

$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process (Resolve-Path $Exe) -ArgumentList $argsList -PassThru
$h = Wait-MainWindow $p
$startupMs = $sw.ElapsedMilliseconds
Start-Sleep -Seconds 3

$idle = Get-ProcStats $p
$idleCpu = Get-IdleCpuMs $p 10

$c0 = (Get-Process -Id $p.Id).TotalProcessorTime.TotalMilliseconds
$t0 = [Diagnostics.Stopwatch]::StartNew()
Send-Key $h Right 6 150; Send-Key $h Left 6 150
Start-Sleep -Milliseconds 400
$navCpu = [math]::Round((Get-Process -Id $p.Id).TotalProcessorTime.TotalMilliseconds - $c0)
$navWall = $t0.ElapsedMilliseconds
if ($Shot) { Start-Sleep -Milliseconds 500; Save-WindowShot $h $Shot }

[XbggBench.U]::ShowWindow($h, 6) | Out-Null   # minimise = "a game is in front"
Start-Sleep -Seconds 1
$bgCpu = Get-IdleCpuMs $p 10
$after = Get-ProcStats $p
Stop-Process -Id $p.Id

[pscustomobject]@{
    StartupToWindowMs   = $startupMs
    IdleForegroundCpuMsPer10s = $idleCpu
    BackgroundCpuMsPer10s     = $bgCpu
    Nav12StepsCpuMs     = $navCpu
    Nav12StepsWallMs    = $navWall
    PrivateMB           = $idle.PrivateMB
    WorkingSetMB        = $idle.WorkingSetMB
    GpuDedicatedMB      = $idle.GpuDedicatedMB
    GpuSharedMB         = $idle.GpuSharedMB
    Threads             = $idle.Threads
    Handles             = $idle.Handles
    ThreadsWhenBackground = $after.Threads
}
