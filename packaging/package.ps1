# Build the Windows release and zip it with the per-user installer.
#
# Produces dist\bevi-knight-<version>-windows.zip containing:
#   bevi-knight.exe, smkdec.exe (film helper), install.ps1, Install.cmd, README.md, docs\*.md
# The zip is the thing to hand a player: unzip, run Install.cmd, and the installer finds or asks
# for the retail folder.
#
#   powershell -ExecutionPolicy Bypass -File packaging\package.ps1
#   powershell -ExecutionPolicy Bypass -File packaging\package.ps1 -SkipBuild

[CmdletBinding()]
param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    $version = (Select-String -Path 'Cargo.toml' -Pattern '^version = "(.+)"$' | Select-Object -First 1).Matches.Groups[1].Value
    if (-not $version) { throw 'could not read the version from Cargo.toml' }
    $name = "bevi-knight-$version-windows"
    $stage = Join-Path $repo "dist\$name"
    $zip = Join-Path $repo "dist\$name.zip"

    if (-not $SkipBuild) {
        Write-Host '==> cargo build --release'
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

        Write-Host '==> building the 32-bit film helper (smkdec)'
        $targets = & rustup target list --installed 2>$null
        if ($targets -contains 'i686-pc-windows-msvc') {
            cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml
            if ($LASTEXITCODE -ne 0) { throw 'the smkdec helper build failed' }
        } else {
            Write-Warning 'i686-pc-windows-msvc is not installed; films will be skipped unless the helper exists.'
            Write-Warning 'Install it with: rustup target add i686-pc-windows-msvc'
        }
    }

    $exe = Join-Path $repo 'target\release\bevi-knight.exe'
    $smk = Join-Path $repo 'tools\smkdec\target\i686-pc-windows-msvc\release\smkdec.exe'
    if (-not (Test-Path -LiteralPath $exe)) { throw "missing $exe" }

    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Path (Join-Path $stage 'docs') -Force | Out-Null
    Copy-Item -LiteralPath $exe -Destination $stage
    if (Test-Path -LiteralPath $smk) { Copy-Item -LiteralPath $smk -Destination $stage }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install.ps1') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Install.cmd') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repo 'README.md') -Destination $stage
    foreach ($doc in @('release-0.1.md', 'release-checklist.md', 'gaps.md')) {
        $src = Join-Path $repo "docs\$doc"
        if (Test-Path -LiteralPath $src) { Copy-Item -LiteralPath $src -Destination (Join-Path $stage 'docs') }
    }

    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
    Compress-Archive -LiteralPath $stage -DestinationPath $zip
    Write-Host ''
    Write-Host "==> $zip"
    Write-Host 'Give the player the zip; they unzip it and run Install.cmd.'
} finally {
    Pop-Location
}
