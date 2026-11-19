<#
.SYNOPSIS
  Builds the release binary and the GitHub release assets in dist\:
  xbgg.exe, xbgg.exe.sha256 and XBGG-v<version>-windows-x64.zip (+ .sha256).
#>
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    $version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    $dist = Join-Path $root 'dist'
    if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
    New-Item -ItemType Directory $dist | Out-Null

    $exe = Join-Path $dist 'xbgg.exe'
    Copy-Item target\release\xbgg.exe $exe
    $fileVersion = (Get-Item $exe).VersionInfo.ProductVersion
    if ($fileVersion -ne $version) { throw "exe reports version $fileVersion, Cargo.toml says $version" }

    function Write-Sha([string]$file) {
        $hash = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
        # sha256sum format, LF line ending, no BOM
        [IO.File]::WriteAllText("$file.sha256", "$hash  $(Split-Path $file -Leaf)`n")
        $hash
    }
    $exeHash = Write-Sha $exe

    $stage = Join-Path $dist "XBGG-v$version"
    New-Item -ItemType Directory $stage | Out-Null
    Copy-Item $exe, README.md, LICENSE, CHANGELOG.md $stage
    $zip = Join-Path $dist "XBGG-v$version-windows-x64.zip"
    Compress-Archive -Path "$stage\*" -DestinationPath $zip
    Remove-Item $stage -Recurse
    $zipHash = Write-Sha $zip

    "version  $version"
    "exe      $exeHash"
    "zip      $zipHash"
    Get-ChildItem $dist | Select-Object Name, Length
}
finally { Pop-Location }
