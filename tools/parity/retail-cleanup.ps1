#requires -Version 7
<#
.SYNOPSIS
  Close a scratch-copy JK.EXE left behind by an interrupted retail capture, without touching a real game.

.DESCRIPTION
  The retail harness (`retail-capture.ps1`) closes its own JK.EXE in a `finally`, but a run that is
  interrupted from outside (operator Ctrl+C, a tool timeout, a crashed shell) can leave the scratch copy
  running, parked in a level. The operator then has to close it by hand — the round-250 QA session had to
  do that repeatedly.

  This script closes only processes whose executable path is inside the harness's own disposable work copy
  (`target/parity/retailcopy` by default). A JK.EXE from anywhere else — the real install, a foreground
  session — is reported and left alone. `retail-capture.ps1` runs the same cleanup automatically before it
  launches (its `Assert-NoConflictingEngine`), so this is the one-liner for after an interrupted run.

.EXAMPLE
  pwsh -NoProfile -File tools/parity/retail-cleanup.ps1
#>
[CmdletBinding()]
param([string]$WorkCopy = 'target/parity/retailcopy')

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))

$workFull = if ([System.IO.Path]::IsPathRooted($WorkCopy)) { $WorkCopy } else { Join-Path (Get-Location) $WorkCopy }
$workFull = [System.IO.Path]::GetFullPath($workFull)

$jk = @(Get-Process -Name 'JK' -ErrorAction SilentlyContinue)
if ($jk.Count -eq 0) {
  Write-Host 'retail-cleanup: no JK.EXE is running'
} else {
  $closed = 0
  foreach ($p in $jk) {
    $path = $null
    try { $path = $p.Path } catch { $path = $null }
    if (-not $path -or -not $path.StartsWith($workFull, [System.StringComparison]::OrdinalIgnoreCase)) {
      Write-Host ("retail-cleanup: JK.EXE pid={0} is not from the scratch copy ({1}); leaving it alone — close it yourself if it is the game" -f $p.Id, $workFull)
      continue
    }
    try {
      $null = $p.CloseMainWindow()
      if (-not $p.WaitForExit(5000)) { $p.Kill(); $null = $p.WaitForExit(5000) }
      $closed++
      Write-Host ("retail-cleanup: closed scratch-copy JK.EXE pid={0}" -f $p.Id)
    } catch {
      Write-Host ("retail-cleanup: could not close scratch-copy JK.EXE pid={0}: {1}" -f $p.Id, $_.Exception.Message)
    }
  }
  Write-Host ("retail-cleanup: {0} scratch-copy process(es) closed" -f $closed)
}

# A force-killed game leaves the primary monitor in its 640x480 mode; the harness's own teardown restores
# the mode but an interruption skipped it. The baseline it saves is the file to restore from.
$baseline = Join-Path (Get-Location) 'target/parity/display-pre.json'
$restore = Join-Path $PSScriptRoot 'restore-display.ps1'
if ((Test-Path -LiteralPath $baseline) -and (Test-Path -LiteralPath $restore)) {
  Write-Host ("retail-cleanup: a pre-run display baseline exists; if the desktop is in 640x480 run: pwsh -NoProfile -File {0}" -f $restore)
}
