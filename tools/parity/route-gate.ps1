#requires -Version 7
<#
.SYNOPSIS
  The level-1 scripted-route gate: the port's fixed-step replay must be deterministic and must match
  the committed route trace.

.DESCRIPTION
  Issue #8's oracle, extended by #15 to the level's own exit. `tools/parity/routes/level1-route.tsv`
  drives the player through the level's own retail-recorded route placements (spawn, cantina, the way
  up, the upper door), then — after the frozen retail fixture's last sample — walks into the shut
  `door0`, presses the player's own use key, walks through the opened doorway, and holds the tunnel and
  landing legs until `01_endlevel.cog` has run. The replay writes
  `tick t_s x y z sector vel_x vel_y vel_z health shields` plus the door/switch cog state, the
  latched events and the mixer's audio state every 600 ms (36 ticks).

  The gate checks seven things:

    1. determinism — the same script replayed twice produces byte-identical traces (retail and port
       must produce the same trace from the same inputs, so the port's own repeatability is a
       precondition);
    2. the exit's own evidence — the plan presses use at both doors and never sends a synthetic
       `activate`, the replay's use trace names the linked things (`use: activated thing 4` and
       `... 47`), `door0_opened` is shut at and before the first use tick and latched after it, the
       body's y then passes the opened leaf (the walk-through crossing), `goal_bin100` reaches 3, and
       `level_end` reaches 1 (the level's script called `jkEndLevel`);
    3. the committed oracle — a replay matches `tools/parity/routetrace/level1-route-port.tsv`
       field for field;
    4. self-check — a copy of the trace with one position moved 0.25 units is **rejected** by the
       comparator, so the gate is known to fail on a perturbed trace and pass on the unmodified one.
       `-Perturb` runs that check; the gate's normal run compares the unmodified trace;
    5. retail parity — the replay matches `level1-route-retail.tsv` within the **epsilon the fixture
       states** (`# epsilon <units>` in its header). Sectors stay exact inside the comparator;
    6. mechanic provenance — `tools/parity/routes/level1-route-provenance.tsv` names the engine
       function and address behind every mechanic the route exercises (walk, floor, collision, use,
       cog messages, damage, AI fire, audio state, level end). A row still on re-derived code must
       name its deviation-ledger entry, and an entry the ledger does not carry fails;
    7. the prerequisite gates — `audio-gate.ps1` (audio state through the same route) and
       `encounter-gate.ps1` (AI fire, damage and the kill order at the door) must both pass. The
       route's own replay runs with `--no-ai`, so combat is measured by the encounter stage inside
       this gate rather than in the same process; `-SkipPrereqs` skips them for iteration and the
       verdict then does not cover them.

  The trace carries the full spawn-to-level-completion route: 43 samples over 1512 ticks, every
  one of them recording position, sector, velocity, both bars, both cogs **and the mixer's audio
  state** — the same columns `--audio-trace` writes, sampled from the same replay tick. No cell is
  `-`: the gate rejects a `-` in either trace, because the comparator would skip it and measure the
  remaining columns only.

  **Coverage is the fixture's own.** The frozen retail capture was taken before #15: it has no
  `door0_opened`/`goal_bin100`/`level_end` columns and its rows stop at tick 684, at the door leg's
  stop pose. The exit segment's first row is the port-only `exit_door` warp at that same tick, so the
  comparator grades exactly the columns that fixture's header carries and the rows **strictly before**
  the warp; the port's exit rows are the port-only segment, proved by check 2 and the port oracle.
  Nothing here needs a new retail capture, and no tolerance is widened.

  Before 2-5, every field of both traces must be present: the comparator skips a `-`, so a required
  health, shield or cog column left as `-` would measure parity for the position columns only and
  pass. The gate rejects any `-` in either trace.

  Both sides capture with the AI **off**: the replay passes `--no-ai` (a capture switch that
  requires `--replay`), and `retail-capture.ps1 -RouteWalk` sets the `-devMode` toggles
  `disableai on` and `noaishots on` (both are toggles, so the capture sets them explicitly and
  re-applies them after every warp). The route's subject is movement and cog state; the cantina
  ambush used to kill retail's player while the port survived, so the bars could never match.
  Combat parity has its own gate (the door encounter).

  `-Epsilon` overrides the fixture's stated value for an experiment; the gate's verdict is the
  fixture's.

.EXAMPLE
  pwsh -File tools/parity/route-gate.ps1
  pwsh -File tools/parity/route-gate.ps1 -Perturb
  pwsh -File tools/parity/route-gate.ps1 -Epsilon 0.5
#>
[CmdletBinding()]
param(
  [string]$Route = 'tools/parity/routes/level1-route.tsv',
  [string]$Level = '01narshadda',
  [string]$PortFixture = 'tools/parity/routetrace/level1-route-port.tsv',
  [string]$RetailFixture = 'tools/parity/routetrace/level1-route-retail.tsv',
  [string]$Out = 'target/parity/routetrace',
  # 0 means "use the epsilon the retail fixture states in its header".
  [double]$Epsilon = 0,
  [switch]$Perturb,
  # Skip the audio and encounter prerequisite gates (iteration only; the verdict then omits them).
  [switch]$SkipPrereqs
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root

function Resolve-PathUnderRoot([string]$Path) {
  if ([System.IO.Path]::IsPathRooted($Path)) { return $Path }
  Join-Path $Root $Path
}

$RoutePath = Resolve-PathUnderRoot $Route
$PortPath = Resolve-PathUnderRoot $PortFixture
$RetailPath = Resolve-PathUnderRoot $RetailFixture
$OutDir = Resolve-PathUnderRoot $Out
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'

if (-not (Test-Path -LiteralPath $Exe)) {
  throw "missing $Exe; run cargo build --release first (the route gate replays the port)"
}
if (-not (Test-Path -LiteralPath $PortPath)) {
  throw "missing the committed port fixture $PortPath"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$runA = Join-Path $OutDir 'level1-route-a.tsv'
$runB = Join-Path $OutDir 'level1-route-b.tsv'
$log = Join-Path $OutDir 'level1-route-port.log'

function Invoke-Replay([string]$Trace) {
  if (Test-Path -LiteralPath $Trace) { Remove-Item -LiteralPath $Trace -Force }
  $env:BK_HEADLESS = '1'
  # `BK_USE_TRACE=1` makes the use key name the linked thing it activated (`use: activated thing N`).
  # The anti-synthetic checks below require that line for both level-1 doors: a plan that reached the
  # cog state some other way cannot produce it.
  $env:BK_USE_TRACE = '1'
  try {
    # `--no-ai`: the route measures movement, not combat. Retail runs the same route under its
    # `-devMode` `disableai` (see `retail-capture.ps1 -RouteWalk`), so both sides fight nothing and
    # the cantina ambush cannot kill one side and not the other.
    & $Exe --start $Level --replay $RoutePath --route-trace $Trace --no-ai --mute *> $log
  } finally {
    Remove-Item Env:BK_HEADLESS -ErrorAction SilentlyContinue
    Remove-Item Env:BK_USE_TRACE -ErrorAction SilentlyContinue
  }
  if (-not (Test-Path -LiteralPath $Trace)) { throw "the replay wrote no trace ($Trace); see $log" }
}

Write-Host 'route gate: replaying the scripted route twice...'
Invoke-Replay $runA
Invoke-Replay $runB

$hashA = (Get-FileHash -LiteralPath $runA -Algorithm SHA256).Hash
$hashB = (Get-FileHash -LiteralPath $runB -Algorithm SHA256).Hash
if ($hashA -ne $hashB) {
  Write-Host "FAIL  determinism: two replays differ ($hashA vs $hashB)"
  exit 1
}
Write-Host ("PASS  determinism: two replays are identical ({0} rows)" -f ((Get-Content -LiteralPath $runA).Count - 1))

# 1b. Required fields. A `-` is a field the comparator would skip, so the gate rejects it before any
# comparison: health, shields and the two cogs are the trace's required fields, and level 1 has both
# cogs. Leaving any of them `-` would measure parity for the position columns only (issue #8 shipped
# that way).
if (-not (Test-Path -LiteralPath $RetailPath)) {
  Write-Host "FAIL  retail: missing $RetailPath"
  exit 1
}
function Find-MissingField([string]$Path) {
  $header = $null
  $lineNo = 0
  foreach ($line in Get-Content -LiteralPath $Path) {
    $lineNo++
    if ($line -match '^\s*(#|$)') { continue }
    $cells = @($line -split "`t")
    if (-not $header) { $header = $cells; continue }
    for ($i = 1; $i -lt $header.Count -and $i -lt $cells.Count; $i++) {
      if ($cells[$i] -eq '-') { return ("line {0}, column '{1}'" -f $lineNo, $header[$i]) }
    }
  }
  return $null
}
$retailMissing = Find-MissingField $RetailPath
if ($retailMissing) {
  Write-Host ("FAIL  retail: {0} leaves a required field as '-' ({1})" -f $RetailFixture, $retailMissing)
  exit 1
}
$portMissing = Find-MissingField $runA
if ($portMissing) {
  Write-Host ("FAIL  port: the replay leaves a required field as '-' ({0})" -f $portMissing)
  exit 1
}
Write-Host 'PASS  required fields: health, shields and both cog states are recorded on both sides'

# 1c. The door opens through the player's own use binding, not a synthetic `activate`. The plan must
# press use (door 0 and the one-way door 47), never send the removed synthetic `switch`, and the
# replay's own use trace must name both linked things. A shortcut that set the cog state cannot
# produce either line.
$planTicks = 0
$planUses = New-Object System.Collections.Generic.List[int]
$planSwitch = $false
foreach ($line in Get-Content -LiteralPath $RoutePath) {
  if ($line -match '^\s*(#|$)') { continue }
  $f = $line -split "`t"
  switch ($f[0]) {
    'input' { $planTicks += [int]$f[1] }
    'wait' { $planTicks += [int]$f[1] }
    'use' { $planUses.Add($planTicks) }
    'switch' { $planSwitch = $true }
  }
}
if ($planSwitch) {
  Write-Host 'FAIL  anti-synthetic: the plan contains a `switch` row (the removed synthetic activate)'
  exit 1
}
if ($planUses.Count -lt 2) {
  Write-Host ('FAIL  anti-synthetic: the plan has {0} use row(s); door0 and the one-way door need the player own use key' -f $planUses.Count)
  exit 1
}
$logText = Get-Content -LiteralPath $log -Raw
foreach ($needle in 'use: activated thing 4', 'use: activated thing 47') {
  if ($logText -notmatch [regex]::Escape($needle)) {
    Write-Host ("FAIL  anti-synthetic: the replay log has no '{0}' line; the door did not open through the use key" -f $needle)
    exit 1
  }
}
# The trace's own shape: the door is shut on every row strictly before the first use tick, and the
# use-tick row is sampled before the key press so it must read shut too; the run ends with both
# events complete.
$traceLines = @(Get-Content -LiteralPath $runA | Where-Object { $_ -notmatch '^\s*(#|$)' })
$traceHeader = @($traceLines[0] -split "`t")
$tickCol = [Array]::IndexOf($traceHeader, 'tick')
$yCol = [Array]::IndexOf($traceHeader, 'y')
$doorCol = [Array]::IndexOf($traceHeader, 'door0_opened')
$goalCol = [Array]::IndexOf($traceHeader, 'goal_bin100')
$endCol = [Array]::IndexOf($traceHeader, 'level_end')
if ($tickCol -lt 0 -or $yCol -lt 0 -or $doorCol -lt 0 -or $goalCol -lt 0 -or $endCol -lt 0) {
  Write-Host 'FAIL  anti-synthetic: the trace has no tick/y/door0_opened/goal_bin100/level_end column'
  exit 1
}
$firstUse = $planUses[0]
$sawOpen = $false
$sawCross = $false
$lastCells = $null
foreach ($line in $traceLines | Select-Object -Skip 1) {
  $cells = @($line -split "`t")
  $lastCells = $cells
  # The use-tick row is sampled before the key press, so it must also read shut; every row strictly
  # before it must be shut. A synthetic trigger that set the cog state early shows `1` at or before
  # the use tick and fails here.
  if ([int]$cells[$tickCol] -le $firstUse -and $cells[$doorCol] -ne '0') {
    Write-Host ("FAIL  anti-synthetic: door0_opened is {0} at tick {1}, at or before the use tick {2}" -f $cells[$doorCol], $cells[$tickCol], $firstUse)
    exit 1
  }
  if ($cells[$doorCol] -eq '1') { $sawOpen = $true }
  # The crossing: after the leaf opened, the body's y passes the leaf (thing 4 at y 9.650073), so the
  # player walked through the doorway the use opened instead of warping past it.
  if ([int]$cells[$tickCol] -gt $firstUse -and $cells[$doorCol] -eq '1' -and [double]$cells[$yCol] -gt 9.650073) {
    $sawCross = $true
  }
}
if (-not $sawOpen -or $lastCells[$doorCol] -ne '1') {
  Write-Host 'FAIL  anti-synthetic: door0_opened never latched'
  exit 1
}
if (-not $sawCross) {
  Write-Host 'FAIL  crossing: no sampled row past the opened door0 leaf (y 9.650073)'
  exit 1
}
if ($lastCells[$goalCol] -ne '3') {
  Write-Host ("FAIL  goal: the last row records goal_bin100 {0}, not 3" -f $lastCells[$goalCol])
  exit 1
}
# `level_end` is `sithMain_SetEndLevel`'s never-cleared latch (`0x4c4bf0`): `01_endlevel.cog`'s
# `user0` sets the goal, sleeps 3 s and calls `jkEndLevel(1)`, so the last sample must see it. A run
# that stops at the sighting fails here, which is why the plan's trailing `wait` outlasts the sleep.
if ($lastCells[$endCol] -ne '1') {
  Write-Host ("FAIL  level end: the last row records level_end {0}; the level's `jkEndLevel` never ran" -f $lastCells[$endCol])
  exit 1
}
Write-Host 'PASS  anti-synthetic: the plan uses the player key, never `switch`, and the replay names things 4 and 47'
Write-Host 'PASS  events: door0_opened latched after the recorded use, the body crossed the leaf, final goal_bin100 = 3 and level_end = 1'

# 1d. Full route coverage and the audio state at every sample (issue #14 AC1/AC3). The trace must
# span the plan from spawn to the final tick (so the extended tail really ran and `jkEndLevel`
# latched), and it must carry the mixer columns. A `-` in any cell was rejected above.
$expectedRows = [int][math]::Floor($planTicks / 36) + 1
# The plan need not end on a sample boundary (the trailing `wait` is a margin), so the last row is
# the last multiple of 36 at or before the plan's final tick.
$expectedLastTick = [int][math]::Floor($planTicks / 36) * 36
$rowCount = $traceLines.Count - 1
if ($rowCount -ne $expectedRows) {
  Write-Host ("FAIL  coverage: the route plan is {0} tick(s), so its {1}-tick samples are {2}; the trace has {3}" -f $planTicks, 36, $expectedRows, $rowCount)
  exit 1
}
if ([int]$lastCells[$tickCol] -ne $expectedLastTick) {
  Write-Host ("FAIL  coverage: the trace's last sample is tick {0}, not the plan's last sample {1} (plan ends at {2})" -f $lastCells[$tickCol], $expectedLastTick, $planTicks)
  exit 1
}
$audioColumns = @('music', 'music_end', 'music_loop', 'music_vol', 'ambient', 'ambient_vol', 'sounds')
$missingAudio = @($audioColumns | Where-Object { $traceHeader -notcontains $_ })
if ($missingAudio.Count -gt 0) {
  Write-Host ("FAIL  audio: the route trace has no {0} column(s)" -f ($missingAudio -join ', '))
  exit 1
}
Write-Host ("PASS  coverage: {0} sample(s) over ticks 0..{1} (spawn to level completion, plan {2} ticks)" -f $rowCount, $expectedLastTick, $planTicks)
Write-Host 'PASS  audio: every sample carries the song, its volume, the audible loop and the window''s one-shots'

# 2. The committed oracle. The comparator is the app's own `--route-compare`; exit code is the gate.
& $Exe --route-compare $PortPath $runA
if ($LASTEXITCODE -ne 0) {
  Write-Host 'FAIL  the replay does not match the committed route fixture (route-compare above)'
  exit 1
}
Write-Host ("PASS  oracle: the replay matches {0}" -f $PortFixture)

# 3. The comparator must reject a trace with one field moved. `-Perturb` is the explicit proof; it is
# also what the roadmap's acceptance asks the gate to demonstrate.
$perturbed = Join-Path $OutDir 'level1-route-perturbed.tsv'
$lines = Get-Content -LiteralPath $runA
$header = @($lines[0] -split "`t")
$xColumn = [Array]::IndexOf($header, 'x')
if ($xColumn -lt 0) { throw 'the route trace has no x column' }
$rows = New-Object System.Collections.Generic.List[string]
$rows.Add($lines[0])
$moved = $false
foreach ($line in $lines | Select-Object -Skip 1) {
  $cells = @($line -split "`t")
  if (-not $moved -and $cells[$xColumn] -ne '-') {
    $cells[$xColumn] = ([double]$cells[$xColumn] + 0.25).ToString('0.0000', [cultureinfo]::InvariantCulture)
    $moved = $true
  }
  $rows.Add(($cells -join "`t"))
}
Set-Content -LiteralPath $perturbed -Value $rows -Encoding utf8

& $Exe --route-compare $PortPath $perturbed 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
  Write-Host 'FAIL  self-check: a trace with x moved 0.25 units passed the comparator'
  exit 1
}
Write-Host 'PASS  self-check: a trace with x moved 0.25 units is rejected'
if ($Perturb) {
  Write-Host 'route gate: the perturbed trace failed and the unmodified trace passed, as required'
}

# 4. Retail parity, at the epsilon the fixture states. The fixture is the oracle, so the tolerance is
# part of it: `# epsilon <units>` is the harness's own resolution (see the file's header).
$stated = Select-String -LiteralPath $RetailPath -Pattern '^\s*#\s*epsilon\s+([0-9.]+)' | Select-Object -First 1
if (-not $stated) {
  Write-Host "FAIL  retail: $RetailFixture states no '# epsilon <units>' tolerance"
  exit 1
}
$fixtureEpsilon = [double]$stated.Matches[0].Groups[1].Value
$tolerance = if ($Epsilon -gt 0) { $Epsilon } else { $fixtureEpsilon }
if ($Epsilon -gt 0 -and $Epsilon -ne $fixtureEpsilon) {
  Write-Host ("note: comparing at {0} (the fixture states {1})" -f $Epsilon, $fixtureEpsilon)
}
# The frozen fixture stops at the door leg's last sample. The exit segment's first row is the
# `exit_door` warp at that same tick, so the replay's row there is the port-only pose, not the pose
# the fixture measured. Grade the fixture over the rows strictly before that warp; the exit rows are
# the plan/event checks' and the port oracle's subject.
$exitWarpTick = $null
$planTick = 0
foreach ($line in Get-Content -LiteralPath $RoutePath) {
  if ($line -match '^\s*(#|$)') { continue }
  $cells = @($line -split "`t" | Where-Object { $_ -ne '' })
  switch ($cells[0]) {
    'input' { $planTick += [int]$cells[1] }
    'wait' { $planTick += [int]$cells[1] }
    'warp' { if ($null -eq $exitWarpTick -and $cells[1] -eq 'exit_door') { $exitWarpTick = $planTick } }
  }
}
if ($null -eq $exitWarpTick) {
  Write-Host 'FAIL  retail: the plan has no `exit_door` warp, so its exit segment is not port-only'
  exit 1
}
$retailSlice = Join-Path $OutDir 'level1-route-retail-slice.tsv'
$sliceHeader = $false
$sliceRows = New-Object System.Collections.Generic.List[string]
foreach ($line in Get-Content -LiteralPath $RetailPath) {
  if ($line -match '^\s*(#|$)') { continue }
  if (-not $sliceHeader) {
    $sliceHeader = $true
    $sliceRows.Add($line)
    continue
  }
  if ([int](($line -split "`t")[0]) -lt $exitWarpTick) { $sliceRows.Add($line) }
}
Set-Content -LiteralPath $retailSlice -Value $sliceRows -Encoding utf8

$env:BK_ROUTE_EPSILON = $tolerance.ToString([cultureinfo]::InvariantCulture)
try {
  & $Exe --route-compare $retailSlice $runA
  $retailOk = $LASTEXITCODE -eq 0
} finally {
  Remove-Item Env:BK_ROUTE_EPSILON -ErrorAction SilentlyContinue
}
if (-not $retailOk) {
  Write-Host ("FAIL  retail: the replay differs from {0} beyond {1}" -f $RetailFixture, $tolerance)
  exit 1
}
Write-Host ("PASS  retail: the replay matches {0} within {1} over the {2} row(s) before the exit warp at tick {3}" -f $RetailFixture, $tolerance, ($sliceRows.Count - 1), $exitWarpTick)

# 5. Mechanic provenance (issue #14 AC2). Every mechanic the route exercises names the engine
# function it was transliterated from, that function's decomp address and the port symbol. A
# mechanic still on re-derived code names its deviation-ledger entry, and the entry must really be
# in the ledger: a deviation cannot be hidden by leaving this file silent about it.
$ProvenancePath = Resolve-PathUnderRoot 'tools/parity/routes/level1-route-provenance.tsv'
if (-not (Test-Path -LiteralPath $ProvenancePath)) {
  Write-Host "FAIL  provenance: missing $ProvenancePath"
  exit 1
}
$RoadmapPath = Resolve-PathUnderRoot 'docs/porting-roadmap.md'
$RequiredMechanics = @('walk', 'floor', 'collision', 'use', 'cog messages', 'damage', 'ai fire', 'audio state', 'level end')
$seenMechanics = @{}
$provenanceRows = 0
foreach ($line in Get-Content -LiteralPath $ProvenancePath) {
  if ($line -match '^\s*(#|$)') { continue }
  $f = @($line -split "`t")
  while ($f.Count -lt 7) { $f += '' }
  $mechanic = $f[0].Trim(); $function = $f[1].Trim(); $address = $f[2].Trim()
  $symbol = $f[3].Trim(); $source = $f[4].Trim(); $status = $f[5].Trim(); $ledger = $f[6].Trim()
  if ($RequiredMechanics -notcontains $mechanic) {
    Write-Host ("FAIL  provenance: mechanic '{0}' is not one the route exercises" -f $mechanic)
    exit 1
  }
  $seenMechanics[$mechanic] = $true
  if (-not $function -or $address -notmatch '^0x[0-9a-fA-F]{6,8}$') {
    Write-Host ("FAIL  provenance: {0} names no engine function/address ('{1}' '{2}')" -f $mechanic, $function, $address)
    exit 1
  }
  if ($status -eq 'ported') {
    $sourcePath = Resolve-PathUnderRoot $source
    if (-not (Test-Path -LiteralPath $sourcePath)) {
      Write-Host ("FAIL  provenance: {0} names the source {1}, which does not exist" -f $mechanic, $source)
      exit 1
    }
    if (-not (Select-String -LiteralPath $sourcePath -Pattern $symbol -SimpleMatch -Quiet)) {
      Write-Host ("FAIL  provenance: {0} names `{1}` ({2}) but {3} has no '{4}'" -f $mechanic, $function, $address, $source, $symbol)
      exit 1
    }
  } elseif ($status -eq 'deviated') {
    if (-not $ledger) {
      Write-Host ("FAIL  provenance: {0} is deviated but names no ledger entry" -f $mechanic)
      exit 1
    }
    if (-not (Select-String -LiteralPath $RoadmapPath -Pattern $ledger -SimpleMatch -Quiet)) {
      Write-Host ("FAIL  provenance: {0} deviates to ledger entry '{1}', which docs/porting-roadmap.md does not carry" -f $mechanic, $ledger)
      exit 1
    }
  } else {
    Write-Host ("FAIL  provenance: {0} has unknown status '{1}' (ported or deviated)" -f $mechanic, $status)
    exit 1
  }
  $provenanceRows++
}
foreach ($mechanic in $RequiredMechanics) {
  if (-not $seenMechanics.ContainsKey($mechanic)) {
    Write-Host ("FAIL  provenance: the route exercises '{0}' but the provenance file does not name it" -f $mechanic)
    exit 1
  }
}
Write-Host ("PASS  provenance: {0} row(s) cover the route's {1} mechanics with the engine function and address" -f $provenanceRows, $RequiredMechanics.Count)

# 6. The prerequisite gates. The route replay runs with `--no-ai`, so combat (AI fire, damage, the
# kill order) and the audio route are measured by their own gates against their own frozen
# references; this gate's verdict includes them.
if ($SkipPrereqs) {
  Write-Host 'SKIP  prerequisites: -SkipPrereqs (the verdict does not cover the audio and encounter gates)'
} else {
  $pwsh = (Get-Command pwsh -ErrorAction SilentlyContinue).Source
  if (-not $pwsh) { $pwsh = (Get-Process -Id $PID).Path }
  foreach ($gate in @('audio-gate.ps1', 'encounter-gate.ps1')) {
    $gatePath = Join-Path $PSScriptRoot $gate
    $gateLog = Join-Path $OutDir (([System.IO.Path]::GetFileNameWithoutExtension($gate)) + '.log')
    Write-Host ("route gate: running the prerequisite {0}..." -f $gate)
    & $pwsh -NoProfile -File $gatePath *> $gateLog
    if ($LASTEXITCODE -ne 0) {
      $tail = @(Get-Content -LiteralPath $gateLog -ErrorAction SilentlyContinue | Select-Object -Last 12) -join "`n"
      Write-Host ("FAIL  prerequisite {0} failed (see {1}):" -f $gate, $gateLog)
      Write-Host $tail
      exit 1
    }
    $passLines = @(Get-Content -LiteralPath $gateLog | Where-Object { $_ -match '^PASS|0 failure' } | Select-Object -Last 3) -join '; '
    Write-Host ("PASS  prerequisite {0}: {1}" -f $gate, $passLines)
  }
}

Write-Host 'route gate: 0 failure(s)'
exit 0
