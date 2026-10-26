# Build the native per-user Windows setup for bevi-knight.
#
# Uses only what Windows ships with: the .NET Framework C# compiler (csc.exe) for the setup and
# uninstaller, and System.Drawing for the icon. No Inno Setup, WiX, NSIS, Squirrel or downloads.
#
# Output: dist\bevi-knight-<version>-setup.exe  (one file: wizard + embedded payload)
#
#   powershell -ExecutionPolicy Bypass -File packaging\build-setup.ps1
#   powershell -ExecutionPolicy Bypass -File packaging\build-setup.ps1 -SkipBuild

[CmdletBinding()]
param([switch]$SkipBuild, [switch]$KeepStaging)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    $version = (Select-String -Path 'Cargo.toml' -Pattern '^version = "(.+)"$' | Select-Object -First 1).Matches.Groups[1].Value
    if (-not $version) { throw 'could not read the version from Cargo.toml' }
    $dist = Join-Path $repo 'dist'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null

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
            Write-Warning 'i686-pc-windows-msvc is not installed; cutscenes will be skipped unless the helper is added.'
            Write-Warning 'Install the target with: rustup target add i686-pc-windows-msvc'
        }
    }

    $exe = Join-Path $repo 'target\release\bevi-knight.exe'
    if (-not (Test-Path -LiteralPath $exe)) { throw "missing $exe (run without -SkipBuild)" }
    $smk = Join-Path $repo 'tools\smkdec\target\i686-pc-windows-msvc\release\smkdec.exe'

    $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    if (-not (Test-Path -LiteralPath $csc)) { throw "the in-box C# compiler was not found at $csc" }
    $fw = Split-Path -Parent $csc
    $refs = @(
        ("/r:" + (Join-Path $fw 'System.dll')),
        ("/r:" + (Join-Path $fw 'System.Windows.Forms.dll')),
        ("/r:" + (Join-Path $fw 'System.Drawing.dll')))
    $zipRefs = @(
        ("/r:" + (Join-Path $fw 'System.IO.Compression.dll')),
        ("/r:" + (Join-Path $fw 'System.IO.Compression.FileSystem.dll')))
    foreach ($ref in $refs + $zipRefs) {
        $path = $ref -replace '^/r:', ''
        if (-not (Test-Path -LiteralPath $path)) { throw "missing reference $path" }
    }

    # Icon: the retail main menu, centre-cropped, as the setup's window icon.
    $icon = Join-Path $dist 'app.ico'
    $sourceIcon = Join-Path $repo 'docs\menu-main.png'
    if (Test-Path -LiteralPath $sourceIcon) {
        Add-Type -AssemblyName System.Drawing
        $image = [System.Drawing.Image]::FromFile($sourceIcon)
        $side = [Math]::Min($image.Width, $image.Height)
        $bitmap = New-Object System.Drawing.Bitmap 64, 64
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $dest = New-Object System.Drawing.Rectangle 0, 0, 64, 64
        $srcRect = New-Object System.Drawing.Rectangle ([int](($image.Width - $side) / 2)), ([int](($image.Height - $side) / 2)), $side, $side
        $graphics.DrawImage($image, $dest, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
        $graphics.Dispose()
        $image.Dispose()
        $handle = $bitmap.GetHicon()
        $ico = [System.Drawing.Icon]::FromHandle($handle)
        $stream = [System.IO.File]::Create($icon)
        $ico.Save($stream)
        $stream.Close()
        $bitmap.Dispose()
        Write-Host "==> icon from docs\menu-main.png"
    } else {
        throw 'docs\menu-main.png is needed for the setup icon'
    }

    # Payload: the game, the film helper and the player-facing docs, zipped into the setup.
    $payload = Join-Path $dist 'setup-payload'
    $zip = Join-Path $dist 'payload.zip'
    foreach ($old in @($payload, $zip)) { if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Recurse -Force } }
    New-Item -ItemType Directory -Path (Join-Path $payload 'docs') -Force | Out-Null
    Copy-Item -LiteralPath $exe -Destination $payload
    if (Test-Path -LiteralPath $smk) {
        Copy-Item -LiteralPath $smk -Destination $payload
    } else {
        Write-Warning 'no smkdec.exe; the installed game will skip cutscenes.'
    }
    Copy-Item -LiteralPath (Join-Path $repo 'README.md') -Destination $payload
    foreach ($doc in @('release-0.1.md', 'release-checklist.md')) {
        $src = Join-Path $repo "docs\$doc"
        if (Test-Path -LiteralPath $src) { Copy-Item -LiteralPath $src -Destination (Join-Path $payload 'docs') }
    }

    Write-Host '==> compiling the uninstaller'
    $uninstaller = Join-Path $payload 'bevi-knight-uninstall.exe'
    & $csc /nologo /target:winexe /optimize+ /platform:anycpu "/win32icon:$icon" "/out:$uninstaller" $refs (Join-Path $PSScriptRoot 'Uninstall.cs')
    if ($LASTEXITCODE -ne 0) { throw 'the uninstaller did not compile' }

    Write-Host '==> packing the payload'
    Compress-Archive -Path (Join-Path $payload '*') -DestinationPath $zip
    $payloadBytes = (Get-Item -LiteralPath $zip).Length

    Write-Host '==> compiling the setup'
    $buildInfo = Join-Path $dist 'BuildInfo.cs'
    [System.IO.File]::WriteAllText($buildInfo,
        "static class BuildInfo { public const string Version = `"$version`"; }`r`n")
    $setup = Join-Path $dist "bevi-knight-$version-setup.exe"
    if (Test-Path -LiteralPath $setup) { Remove-Item -LiteralPath $setup -Force }
    & $csc /nologo /target:winexe /optimize+ /platform:anycpu "/win32icon:$icon" "/out:$setup" `
        "/resource:$zip,payload.zip" $refs $zipRefs (Join-Path $PSScriptRoot 'Setup.cs') $buildInfo
    if ($LASTEXITCODE -ne 0) { throw 'the setup did not compile' }

    if (-not $KeepStaging) {
        foreach ($old in @($payload, $zip, $buildInfo, $icon)) {
            if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Recurse -Force }
        }
    }

    $hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
    $size = (Get-Item -LiteralPath $setup).Length
    Write-Host ''
    Write-Host "==> $setup"
    Write-Host ("    {0:N1} MB (payload {1:N1} MB), sha256 {2}" -f ($size / 1MB), ($payloadBytes / 1MB), $hash)
    Write-Host '    Run it to install, or: bevi-knight-<version>-setup.exe /S /GAMEPATH="<game folder>" /DIR="<dir>"'
} finally {
    Pop-Location
}
