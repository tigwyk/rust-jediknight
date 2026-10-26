# bevi-knight per-user installer.
#
# Finds the retail Star Wars Jedi Knight: Dark Forces II install (GOG registry, then the usual
# GOG folders, then asks), copies the release build beside it, saves the game path for the game to
# read, and makes a Start Menu shortcut. Per-user: no admin, no registry writes, no game files are
# touched or copied. Uninstall with -Uninstall or the uninstall.ps1 it writes.
#
#   powershell -ExecutionPolicy Bypass -File packaging\install.ps1
#   powershell -ExecutionPolicy Bypass -File packaging\install.ps1 -GamePath "C:\GOG Games\..." -NoShortcuts
#
# It is also the installed uninstaller's engine: uninstall.ps1 calls this file with -Uninstall.

[CmdletBinding()]
param(
    [string]$GamePath,
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\bevi-knight'),
    [switch]$NoShortcuts,
    [switch]$DesktopShortcut,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

function Write-Step($text) { Write-Host "==> $text" }
function Write-Warn2($text) { Write-Host "!!  $text" -ForegroundColor Yellow }

# Does this folder hold the retail game? The engine executable and the Resource folder are what
# the port loads; a GOG Games root that merely contains the title in its path must not match.
function Test-JkInstall([string]$Dir) {
    if ([string]::IsNullOrWhiteSpace($Dir)) { return $false }
    $Dir = $Dir.Trim().Trim('"')
    if (-not (Test-Path -LiteralPath $Dir -PathType Container)) { return $false }
    $exe = Get-ChildItem -LiteralPath $Dir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^(JK|JediKnight|JediKnight2)\.EXE$' } |
        Select-Object -First 1
    if (-not $exe) { return $false }
    return (Test-Path -LiteralPath (Join-Path $Dir 'Resource') -PathType Container)
}

# The same places the game itself looks: GOG's registry entries and the folders GOG installs to.
function Find-JkInstall {
    $candidates = New-Object System.Collections.Generic.List[string]
    foreach ($key in @(
            'HKLM:\SOFTWARE\WOW6432Node\GOG.com\Games',
            'HKLM:\SOFTWARE\GOG.com\Games',
            'HKCU:\SOFTWARE\GOG.com\Games')) {
        if (-not (Test-Path -LiteralPath $key)) { continue }
        foreach ($entry in Get-ChildItem -LiteralPath $key -ErrorAction SilentlyContinue) {
            try { $path = $entry.GetValue('path') } catch { $path = $null }
            if ($path) { $candidates.Add([string]$path) }
        }
    }
    foreach ($drive in [char[]]('CDEFGHIJKLMNOPQRSTUVWXYZ')) {
        foreach ($root in @(
                "${drive}:\GOG Games",
                "${drive}:\Games",
                "${drive}:\Program Files (x86)\GOG Galaxy\Games",
                "${drive}:\Program Files\GOG Galaxy\Games")) {
            if (-not (Test-Path -LiteralPath $root -PathType Container)) { continue }
            foreach ($dir in Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue) {
                $candidates.Add($dir.FullName)
            }
        }
    }
    foreach ($candidate in $candidates) {
        if (Test-JkInstall $candidate) { return $candidate.Trim().Trim('"') }
    }
    return $null
}

function Get-ShortcutPaths {
    $paths = @()
    $start = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
    $paths += (Join-Path $start 'bevi-knight.lnk')
    $desktop = [Environment]::GetFolderPath('Desktop')
    if ($desktop) { $paths += (Join-Path $desktop 'bevi-knight.lnk') }
    return $paths
}

function Remove-Shortcuts {
    foreach ($shortcut in Get-ShortcutPaths) {
        if (Test-Path -LiteralPath $shortcut) {
            Remove-Item -LiteralPath $shortcut -Force
            Write-Step "removed $shortcut"
        }
    }
}

if ($Uninstall) {
    Write-Step "Removing bevi-knight from $InstallDir"
    Remove-Shortcuts
    if (Test-Path -LiteralPath $InstallDir) {
        Remove-Item -LiteralPath $InstallDir -Recurse -Force
        Write-Step "removed $InstallDir"
    }
    $config = Join-Path $env:APPDATA 'bevi-knight\game-path.txt'
    if (Test-Path -LiteralPath $config) {
        Remove-Item -LiteralPath $config -Force
        Write-Step "removed saved game path $config"
    }
    $configDir = Join-Path $env:APPDATA 'bevi-knight'
    if ((Test-Path -LiteralPath $configDir) -and -not (Get-ChildItem -LiteralPath $configDir -Force)) {
        Remove-Item -LiteralPath $configDir -Force
    }
    Write-Step 'Done. The game install itself was not touched.'
    return
}

# --- find the build to install -------------------------------------------------------------
$scriptDir = $PSScriptRoot
$parent = Split-Path -Parent $scriptDir
$exeCandidates = @(
    (Join-Path $scriptDir 'bevi-knight.exe'),
    (Join-Path $parent 'target\release\bevi-knight.exe'))
$exe = $exeCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $exe) {
    throw "bevi-knight.exe not found. Build it first: cargo build --release (or use the packaged zip, which has it beside this script)."
}
$smkCandidates = @(
    (Join-Path $scriptDir 'smkdec.exe'),
    (Join-Path $parent 'tools\smkdec\target\i686-pc-windows-msvc\release\smkdec.exe'))
$smk = $smkCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

# --- find the retail game ------------------------------------------------------------------
if (-not (Test-JkInstall $GamePath)) {
    if ($GamePath) {
        Write-Warn2 "$GamePath is not a Jedi Knight install (no JK.EXE or no Resource folder)."
    }
    Write-Step 'Searching the GOG registry and the usual GOG folders...'
    $found = Find-JkInstall
    if ($found) {
        $GamePath = $found
    } else {
        Write-Host ''
        Write-Host 'Could not find Star Wars Jedi Knight: Dark Forces II automatically.'
        Write-Host 'Paste the folder that contains JK.EXE (for example C:\GOG Games\Star Wars Jedi Knight - Dark Forces 2).'
        while (-not (Test-JkInstall $GamePath)) {
            if ($GamePath) { Write-Warn2 "That is not a Jedi Knight install: $GamePath" }
            $GamePath = (Read-Host 'Install folder (Enter to cancel)').Trim().Trim('"')
            if ([string]::IsNullOrWhiteSpace($GamePath)) { throw 'No install selected.' }
        }
    }
}
$GamePath = $GamePath.Trim().Trim('"')
Write-Step "Game: $GamePath"

# --- install -------------------------------------------------------------------------------
Write-Step "Installing to $InstallDir"
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $InstallDir 'bevi-knight.exe') -Force
if ($smk) {
    Copy-Item -LiteralPath $smk -Destination (Join-Path $InstallDir 'smkdec.exe') -Force
} else {
    Write-Warn2 'The film helper was not found, so cutscenes will be skipped.'
    Write-Warn2 'Build it with: rustup target add i686-pc-windows-msvc; cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml'
}

# The game reads this file beside its executable first (see src/gamepath.rs).
[IO.File]::WriteAllText((Join-Path $InstallDir 'game-path.txt'), $GamePath + "`r`n")

# Docs travel with the install so the caveats are next to the program.
$docs = Join-Path $InstallDir 'docs'
New-Item -ItemType Directory -Path $docs -Force | Out-Null
$docSources = @()
$stagedDocs = Join-Path $scriptDir 'docs'
if (Test-Path -LiteralPath $stagedDocs -PathType Container) {
    $docSources += Get-ChildItem -LiteralPath $stagedDocs -File -Filter '*.md'
} elseif (Test-Path -LiteralPath (Join-Path $parent 'docs') -PathType Container) {
    $docSources += Get-ChildItem -LiteralPath (Join-Path $parent 'docs') -File -Filter '*.md'
}
$readme = Join-Path $parent 'README.md'
if (-not (Test-Path -LiteralPath $readme) -and (Test-Path -LiteralPath (Join-Path $scriptDir 'README.md'))) {
    $readme = Join-Path $scriptDir 'README.md'
}
foreach ($doc in $docSources) { Copy-Item -LiteralPath $doc.FullName -Destination $docs -Force }
if ($readme -and (Test-Path -LiteralPath $readme)) {
    Copy-Item -LiteralPath $readme -Destination (Join-Path $docs 'README.md') -Force
}

# The uninstaller is this script in uninstall mode.
$self = Join-Path $scriptDir 'install.ps1'
if (Test-Path -LiteralPath $self) {
    Copy-Item -LiteralPath $self -Destination (Join-Path $InstallDir 'install.ps1') -Force
}
$uninstallFile = Join-Path $InstallDir 'uninstall.ps1'
$uninstallBody = @'
param([string]$InstallDir = $PSScriptRoot)
& (Join-Path $PSScriptRoot 'install.ps1') -Uninstall -InstallDir $InstallDir
Read-Host 'Press Enter to close'
'@
[IO.File]::WriteAllText($uninstallFile, $uninstallBody + "`r`n")
$uninstallCmd = Join-Path $InstallDir 'Uninstall.cmd'
[IO.File]::WriteAllText($uninstallCmd, "@echo off`r`npowershell -NoProfile -ExecutionPolicy Bypass -File `"%~dp0uninstall.ps1`"`r`n")

# --- shortcuts -----------------------------------------------------------------------------
if (-not $NoShortcuts) {
    $ws = New-Object -ComObject WScript.Shell
    $startDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
    $target = Join-Path $InstallDir 'bevi-knight.exe'
    $lnk = $ws.CreateShortcut((Join-Path $startDir 'bevi-knight.lnk'))
    $lnk.TargetPath = $target
    $lnk.WorkingDirectory = $InstallDir
    $lnk.Description = 'Star Wars Jedi Knight: Dark Forces II reconstruction'
    $lnk.Save()
    Write-Step "Start Menu shortcut: $(Join-Path $startDir 'bevi-knight.lnk')"
    if ($DesktopShortcut) {
        $desktop = [Environment]::GetFolderPath('Desktop')
        $deskLnk = $ws.CreateShortcut((Join-Path $desktop 'bevi-knight.lnk'))
        $deskLnk.TargetPath = $target
        $deskLnk.WorkingDirectory = $InstallDir
        $deskLnk.Description = 'Star Wars Jedi Knight: Dark Forces II reconstruction'
        $deskLnk.Save()
        Write-Step "Desktop shortcut: $(Join-Path $desktop 'bevi-knight.lnk')"
    }
}

Write-Host ''
Write-Step 'Done.'
Write-Host "  Game found:  $GamePath"
Write-Host "  Installed:   $InstallDir"
Write-Host "  Run:         $InstallDir\bevi-knight.exe"
Write-Host '  Uninstall:   the Uninstall.cmd in that folder (or install.ps1 -Uninstall)'
Write-Host '  The game install itself was not modified.'
