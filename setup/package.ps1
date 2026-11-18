# Builds the release and the portable zip into dist\:
#   dist\CursorDrop-v<version>-win64.zip          (CursorDrop\CursorDrop.exe + cursors\ + texts)
#   dist\CursorDrop-v<version>-win64.zip.sha256
# The version comes from Cargo.toml. Run from anywhere: powershell -File setup\package.ps1
# (ASCII only on purpose: Windows PowerShell 5 reads .ps1 files as ANSI.)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# cargo isn't on PATH by default on this machine (rustup lives under scoop\persist).
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $rustup = Join-Path $env:USERPROFILE 'scoop\persist\rustup'
    if (-not (Test-Path (Join-Path $rustup '.cargo\bin\cargo.exe'))) { throw 'cargo not found' }
    $env:PATH = (Join-Path $rustup '.cargo\bin') + ';' + $env:PATH
    $env:RUSTUP_HOME = Join-Path $rustup '.rustup'
    $env:CARGO_HOME = Join-Path $rustup '.cargo'
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$name = "CursorDrop-v$version-win64"
Write-Host "Packaging $name"

cargo test --release
if ($LASTEXITCODE -ne 0) { throw 'tests failed' }
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

$dist = Join-Path $root 'dist'
$staging = Join-Path $dist $name
$app = Join-Path $staging 'CursorDrop'
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory -Force (Join-Path $app 'cursors') | Out-Null

Copy-Item 'target\release\CursorDrop.exe' $app
Copy-Item 'setup\README.txt' (Join-Path $app 'README.txt')
Copy-Item 'LICENSE' (Join-Path $app 'LICENSE.txt')
Copy-Item 'THIRD_PARTY_NOTICES.md' (Join-Path $app 'THIRD_PARTY_NOTICES.txt')
# A placeholder so the (otherwise empty) cursors folder survives zipping; CursorDrop ignores it.
Set-Content -Encoding ASCII (Join-Path $app 'cursors\Put cursor packs here.txt') `
    'Drop .zip, .7z or .rar cursor packs into this folder - CursorDrop applies them automatically.'

# The exe must report the version we are about to publish.
$reported = & (Join-Path $app 'CursorDrop.exe') --version | Out-String
if ($reported.Trim() -ne "CursorDrop $version") { throw "exe reports '$($reported.Trim())', expected 'CursorDrop $version'" }

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $app -DestinationPath $zip
Remove-Item -Recurse -Force $staging

$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
Set-Content -Encoding ASCII "$zip.sha256" "$hash  $name.zip"

Write-Host ''
Write-Host "Built $zip"
Write-Host "SHA-256 $hash"
