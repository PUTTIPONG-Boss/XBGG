<#
.SYNOPSIS
  End-to-end test of the in-app updater against the latest GitHub release.
  A copy of xbgg.exe (made different by appending bytes) runs as a pretend
  older version, installs the update via U + Enter, and must restart as the
  exact release binary.
#>
param([string]$Exe = "", [string]$PretendVersion = "0.0.1")
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\target\release\xbgg.exe' }
. "$PSScriptRoot\winapi.ps1"
function Log($m) { "[{0:HH:mm:ss}] {1}" -f (Get-Date), $m }

$rel = Invoke-RestMethod 'https://api.github.com/repos/PUTTIPONG-Boss/XBGG/releases/latest' -Headers @{ 'User-Agent' = 'XBGG-update-test' }
$shaUrl = ($rel.assets | Where-Object name -eq 'xbgg.exe.sha256').browser_download_url
$content = (Invoke-WebRequest $shaUrl -UseBasicParsing -Headers @{ 'User-Agent' = 'XBGG-update-test' }).Content
if ($content -is [byte[]]) { $content = [Text.Encoding]::ASCII.GetString($content) }
$expected = ($content.Trim() -split '\s+')[0].ToLower()
Log "latest release $($rel.tag_name), xbgg.exe sha256 $expected"

$dir = Join-Path ([IO.Path]::GetTempPath()) "xbgg-update-test-$PID"
New-Item -ItemType Directory $dir -Force | Out-Null
$target = Join-Path $dir 'xbgg.exe'
Copy-Item (Resolve-Path $Exe) $target
# Make the "old" binary differ from the release (bytes after the PE image are ignored by Windows).
$fs = [IO.File]::Open($target, 'Append'); $fs.Write([byte[]](1..16), 0, 16); $fs.Close()
$before = (Get-FileHash $target).Hash.ToLower()
Log "old binary sha256 $before"

$result = 'FAIL'
$procs = @()
$noBoost = Disable-XbggBoost
try {
    # CLI check first.
    $env:XBGG_PRETEND_VERSION = $PretendVersion
    $out = Join-Path $dir 'check.txt'
    Start-Process $target -ArgumentList '--check-update' -Wait -NoNewWindow -RedirectStandardOutput $out
    Log "--check-update: $(Get-Content $out)"

    $p = Start-Process $target -ArgumentList '--windowed' -PassThru
    Remove-Item Env:XBGG_PRETEND_VERSION
    $procs += $p
    $h = Wait-MainWindow $p
    Start-Sleep -Seconds 6               # startup update check
    Send-Key $h F2 1 400                 # open the menu ("Install update" is the first row)
    Send-Key $h Enter 1 400              # choose it: opens the confirmation
    Send-Key $h Enter 1 100              # confirm
    Log 'update confirmed in the UI'

    if (-not $p.WaitForExit(120000)) { throw 'old process did not exit after the update' }
    Log "old process exited (code $($p.ExitCode))"

    $deadline = (Get-Date).AddSeconds(20); $new = $null
    while (-not $new -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        $new = Get-Process xbgg -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $target } | Select-Object -First 1
    }
    if (-not $new) { throw 'updated XBGG did not restart' }
    $procs += $new
    Log "restarted as pid $($new.Id)"
    Start-Sleep -Seconds 5

    $after = (Get-FileHash $target).Hash.ToLower()
    Log "new binary sha256 $after"
    if ($after -ne $expected) { throw 'installed binary does not match the release checksum' }
    if ($after -eq $before) { throw 'binary was not replaced' }
    $old = Test-Path (Join-Path $dir 'xbgg.exe.old')
    $tmp = Test-Path (Join-Path $dir 'xbgg.exe.new')
    Log "leftovers: xbgg.exe.old=$old xbgg.exe.new=$tmp"
    if ($old -or $tmp) { throw 'leftover files after update' }
    $result = 'PASS'
}
finally {
    foreach ($q in $procs) { if (-not $q.HasExited) { Stop-Process -Id $q.Id -ErrorAction SilentlyContinue } }
    Get-Process xbgg -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $target } | Stop-Process -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
    Restore-XbggConfig $noBoost
}
Log "RESULT: $result"
