#requires -Version 7
<#
.SYNOPSIS
  Put the desktop display mode back after an interrupted retail capture.

.DESCRIPTION
  The harness closes the retail game with WM_CLOSE so the engine itself restores the monitor, and
  retail-capture.ps1 also stores the desktop modes in target/parity/display-pre.json as a safety net.
  If a run was interrupted (Ctrl+C, crash, task-manager kill, power loss), the primary monitor can be
  left in the game's 640x480 mode. Run this script to restore it:

    (default)          the modes saved before the last capture run
    -RegistryDefaults  the modes Windows has stored as the desktop default (what it boots with)

  This is a session-only change; it does not rewrite any saved display configuration.

.EXAMPLE
  pwsh -File tools/parity/restore-display.ps1

.EXAMPLE
  pwsh -File tools/parity/restore-display.ps1 -RegistryDefaults
#>
[CmdletBinding()]
param(
    [string]$State = 'target/parity/display-pre.json',
    [switch]$RegistryDefaults
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not [System.IO.Path]::IsPathRooted($State)) { $State = Join-Path $repoRoot $State }
. (Join-Path $PSScriptRoot 'display.ps1')

if (Get-Process -Name 'JK' -ErrorAction SilentlyContinue) {
    throw 'JK.EXE is running. Close the game first; the engine restores the display mode itself on exit.'
}

Write-Host ('display before: ' + [JkDisplay]::Summary())

if ($RegistryDefaults) {
    $result = Restore-JkRegistryDefaults
} elseif (Test-Path -LiteralPath $State) {
    $saved = (Get-Content -LiteralPath $State -Raw | ConvertFrom-Json).modes
    if (Test-JkDisplayMatches -Saved $saved) {
        Write-Host 'display already matches the saved baseline; nothing to do'
        exit 0
    }
    $result = Restore-JkDisplayState -Saved $saved
} else {
    Write-Host "no saved baseline at $State; restoring the Windows display default instead"
    $result = Restore-JkRegistryDefaults
}

Write-Host ('applied:       ' + ($result.applied -join ', '))
Write-Host ('display after: ' + [JkDisplay]::Summary())

if (-not $RegistryDefaults -and (Test-Path -LiteralPath $State)) {
    $saved = (Get-Content -LiteralPath $State -Raw | ConvertFrom-Json).modes
    if (-not (Test-JkDisplayMatches -Saved $saved)) {
        Write-Host 'WARNING: the saved baseline could not be applied; try -RegistryDefaults'
        exit 1
    }
}
