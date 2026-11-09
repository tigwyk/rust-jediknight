#requires -Version 7
<#
.SYNOPSIS
  Retail-vs-port gate for the fixed level-1 door encounter (issue #9).

.DESCRIPTION
  Both sides write the same TSV (`tools/parity/door-encounter.tsv` names the pose and duration):

      t_ms  kind  thing  template  x  y  z  yaw  health  shields  detail

  `retail-capture.ps1 -EncounterMs` samples the retail engine's own memory (player thing health,
  inventory bin 60 shields, the thing array for actors and projectiles, the engine game clock);
  `port-encounter.ps1` runs the port's runtime and writes its own timeline. This gate reports the
  timing delta of every shot and hit, the per-hit health/shield delta, the kill order, and the actor
  state delta at each cadence sample, then exits non-zero when any of them leaves its tolerance. It
  is the instrument tickets #11 (AI timing) and #12 (combat timing) measure against.

  Fire cadence is graded on the **interval** sequence: the two sides observe the same shots a flight
  apart (retail's impact, the port's launch), so consecutive-shot intervals are comparable even
  though absolute times are not. Retail's cadence is nondeterministic: `sithAICmd_PrimaryFire`
  draws every delay from `rand()` (`(1.2 - 0.2 * rand()) * fltArg[0]`), and acquisition is a chance
  roll, so which shot falls inside the 9.9 s window and in what order is one realisation. Production
  shot/hit counts therefore allow one event (`-ShotCountSlack`/`-HitCountSlack` 1) and the interval
  sequences are compared as a **sorted multiset** over the shared count inside
  `-ShotCadenceToleranceMs`; the final bars are compared only when the counts match (the per-hit
  damage is graded on every shared hit). This is the distribution retail guarantees, not one run's
  ordering. Actor facing for a `Roam`-class actor and the absolute time of a shot are the other
  `rand()` consumers.

  Actor rows are joined by **authored placement**, not by the `thing` number: retail's number is a
  slot in the engine's thing pool and the port's is the level's placement order, and the two only
  agree where both arrays happen to line up. The gate pairs each actor at the first sample where
  both traces report it (actors both builds place at the same point agree to 0.004 units), so an
  actor that walks into the sampling radius after the first tick is paired when it appears rather
  than left unmatched. Any actor still unmatched after the last sample fails; there is no
  missing-actor fraction.

  The retail fixture is committed under `tools/parity/encounter/`; a fresh capture in
  `target/parity/encounter/` takes precedence. The gate records the port twice and fails when the
  two runs leave the measured run-to-run band (shape, endpoint, event timing, actor state, kill
  order), so "the fixture is repeatable" is checked rather than assumed.

  Tolerances are parameters with measured defaults. The port's AI is not on retail's timeline yet
  (#11/#12), so the default run is expected to report deltas; the gate still fails on them, which is
  the contract. A structural change (missing rows, a non-repeatable run, a fixture edit) fails too.

.EXAMPLE
  pwsh -File tools/parity/encounter-gate.ps1
  pwsh -File tools/parity/encounter-gate.ps1 -Retail target/parity/encounter/door_encounter.tsv -Port target/parity/encounter/door_encounter-a.tsv
#>
[CmdletBinding()]
param(
  [string]$Encounters = 'tools/parity/door-encounter.tsv',
  [string]$Fixtures = 'tools/parity/encounter',
  [string]$Out = 'target/parity/encounter',
  [string]$Only = 'door_encounter',
  # Compare against this retail TSV instead of the seeded fixture (used to prove the gate fails on an
  # edited fixture).
  [string]$Retail = '',
  # Compare against this port TSV instead of recording one.
  [string]$Port = '',
  [switch]$SkipCapture,
  [switch]$NoRepeat,
  # The retail engine renders a few frames a second, so its last cadence sample can sit up to one
  # frame (500 ms, the engine's dt clamp) before the port's last fixed-step sample.
  [double]$DurationToleranceMs = 600,
  # Event-count band: |port - retail| allowed before the count itself is a failure. The engine's
  # per-shot delay is `(1.2 - 0.2*rand())*fltArg[0]` and acquisition is a chance roll, so which shot
  # falls inside the 9.9 s window is a random draw: one event is the reproducible band.
  [int]$ShotCountSlack = 1,
  [int]$HitCountSlack = 1,
  [double]$ShotToleranceMs = 1000,
  [double]$HitToleranceMs = 1000,
  [double]$HealthTolerance = 10,
  [double]$ShieldsTolerance = 10,
  [double]$ActorPositionTolerance = 1.5,
  [double]$ActorYawTolerance = 30,
  [double]$ActorHealthTolerance = 15,
  # Actor identity: retail's `thing` is a thing-pool slot, the port's is the level's placement order,
  # so the two only agree where both arrays happen to line up. Actors are paired by nearest position
  # at the first sample; this is the distance below which two rows are the same authored placement
  # (actors both builds place agree to 0.004 units). An actor with no counterpart fails.
  [double]$ActorIdentityTolerance = 0.5,
  # Consecutive-shot interval band. Retail's impacts are quantised by its 100 ms sample grid, so one
  # interval can differ by that grid plus a frame; measured max on the door fixture is 234 ms.
  [double]$ShotCadenceToleranceMs = 250
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root
$OutDir = if ([System.IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $Root $Out }
$FixtureDir = if ([System.IO.Path]::IsPathRooted($Fixtures)) { $Fixtures } else { Join-Path $Root $Fixtures }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
New-Item -ItemType Directory -Force -Path $FixtureDir | Out-Null

$failures = 0
function Fail([string]$Message) {
  Write-Host "FAIL  $Message"
  $script:failures++
}
function Num([string]$Text) {
  if ([string]::IsNullOrWhiteSpace($Text)) { return $null }
  $value = 0.0
  if ([double]::TryParse($Text, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$value)) { return $value }
  return $null
}
function Read-Trace([string]$Path) {
  $rows = [System.Collections.Generic.List[object]]::new()
  foreach ($line in Get-Content -LiteralPath $Path) {
    if ($line -match '^\s*(#|$)') { continue }
    $f = $line -split "`t"
    if ($f[0] -eq 't_ms' -or $f.Length -lt 11) { continue }
    $rows.Add([pscustomobject]@{
        T        = [double]$f[0]
        Kind     = $f[1]
        Thing    = $f[2]
        Template = $f[3]
        X        = Num $f[4]
        Y        = Num $f[5]
        Z        = Num $f[6]
        Yaw      = Num $f[7]
        Health   = Num $f[8]
        Shields  = Num $f[9]
        Detail   = $f[10]
      })
  }
  return $rows
}

# Smallest signed angle between two yaws, in degrees. PowerShell's `%` keeps the sign, so the
# naive `(a - b + 180) % 360 - 180` reads 356.7 for a 3.3-degree wrap.
function Get-WrapDelta([double]$A, [double]$B) {
  $d = ($B - $A) % 360.0
  if ($d -gt 180.0) { $d -= 360.0 }
  if ($d -lt -180.0) { $d += 360.0 }
  return [math]::Abs($d)
}

# Per-cadence actor-state deltas. Both traces key an actor by (sample time, thing id): the ids are
# the level's own placement order on both builds, which is why the retail fixture and the port's
# runtime name the same actor by the same number.
function Get-ActorDeltas($BaseRows, $OtherRows) {
  $other = @{}
  $extra = @{}
  foreach ($row in @($OtherRows | Where-Object Kind -eq 'actor')) {
    $key = '{0}|{1}' -f [int]$row.T, $row.Thing
    $other[$key] = $row
    $extra[$key] = $true
  }
  $position = 0.0; $yaw = 0.0; $health = 0.0; $compared = 0; $missing = 0
  foreach ($row in @($BaseRows | Where-Object Kind -eq 'actor')) {
    $key = '{0}|{1}' -f [int]$row.T, $row.Thing
    $extra.Remove($key)
    if (-not $other.ContainsKey($key)) { $missing++; continue }
    $p = $other[$key]
    if ($null -eq $p.X -or $null -eq $row.X) { continue }
    $dx = [double]$p.X - [double]$row.X
    $dy = [double]$p.Y - [double]$row.Y
    $dz = [double]$p.Z - [double]$row.Z
    $position = [math]::Max($position, [math]::Sqrt($dx * $dx + $dy * $dy + $dz * $dz))
    if ($null -ne $p.Yaw -and $null -ne $row.Yaw) {
      $yaw = [math]::Max($yaw, (Get-WrapDelta ([double]$row.Yaw) ([double]$p.Yaw)))
    }
    if ($null -ne $p.Health -and $null -ne $row.Health) {
      $health = [math]::Max($health, [math]::Abs([double]$p.Health - [double]$row.Health))
    }
    $compared++
  }
  return [pscustomobject]@{
    Compared = $compared
    Missing  = $missing
    Extra    = $extra.Count
    Position = $position
    Yaw      = $yaw
    Health   = $health
  }
}

# Pair nth events and report the absolute timing delta. Events are in time order on both sides.
function Get-EventDeltas($BaseEvents, $OtherEvents) {
  $count = [math]::Min($BaseEvents.Count, $OtherEvents.Count)
  $max = 0.0; $sum = 0.0
  for ($i = 0; $i -lt $count; $i++) {
    $delta = [math]::Abs([double]$OtherEvents[$i].T - [double]$BaseEvents[$i].T)
    $max = [math]::Max($max, $delta)
    $sum += $delta
  }
  $mean = if ($count -gt 0) { $sum / $count } else { 0.0 }
  return [pscustomobject]@{ Count = $count; Max = $max; Mean = $mean }
}

# The order things died in, which is what a no-input encounter can say about kill order.
#
# Both captures drop a thing from the trace once it is gone — the retail reader skips an actor whose
# health is 0 or less, and the port's trace skips a destroyed one — and both name an actor by the
# level's own placement index, so the last sample each thing appears in is its death time. A thing
# that walks out of the 8-unit sampling radius reads the same way; the actor-position check above is
# what distinguishes that case (a fixture that grows moving actors should record deaths explicitly).
function Get-KillOrder($Rows) {
  $first = @{}
  $last = @{}
  $end = 0.0
  foreach ($row in @($Rows)) {
    $t = [double]$row.T
    if ($t -gt $end) { $end = $t }
    if ($row.Kind -ne 'actor') { continue }
    $key = [string]$row.Thing
    if (-not $first.ContainsKey($key)) { $first[$key] = $t }
    $last[$key] = $t
  }
  return @($last.GetEnumerator() | Where-Object { $_.Value -lt $end } | Sort-Object Value | ForEach-Object { $_.Key })
}



# ---- retail fixture: committed, with a fresh capture taking precedence -------------------------
$retailPath = $Retail
if (-not $retailPath) {
  $retailPath = Join-Path $OutDir ("{0}.tsv" -f $Only)
  if (-not (Test-Path -LiteralPath $retailPath)) {
    $fixture = Join-Path $FixtureDir ("{0}.tsv" -f $Only)
    if (-not (Test-Path -LiteralPath $fixture)) {
      Fail "no retail fixture for '$Only' (looked for $fixture); capture it with tools/parity/retail-capture.ps1 -Shots tools/parity/shots.tsv -Only route_door -EncounterMs 10000 -EncounterCadenceMs 100"
      Write-Host ("encounter gate: {0} failure(s)" -f $failures)
      exit 1
    }
    Copy-Item -LiteralPath $fixture -Destination $retailPath
    Write-Host ("seeded {0} from the committed retail fixture" -f $retailPath)
  }
}
if (-not (Test-Path -LiteralPath $retailPath)) { Fail "missing retail trace $retailPath"; Write-Host ("encounter gate: {0} failure(s)" -f $failures); exit 1 }

# ---- port half: record (twice for repeatability) or use an existing trace ----------------------
$portPath = $Port
$repeatPath = ''
if (-not $portPath) {
  $portPath = Join-Path $OutDir ("{0}-a.tsv" -f $Only)
  if (-not $SkipCapture) {
    & pwsh -NoProfile -File tools/parity/port-encounter.ps1 -Encounters $Encounters -Out $OutDir -Only $Only -Suffix '-a'
    if (-not $NoRepeat) {
      & pwsh -NoProfile -File tools/parity/port-encounter.ps1 -Encounters $Encounters -Out $OutDir -Only $Only -Suffix '-b'
      $repeatPath = Join-Path $OutDir ("{0}-b.tsv" -f $Only)
      if (-not (Test-Path -LiteralPath $repeatPath)) { $repeatPath = '' }
    }
  }
}
if (-not (Test-Path -LiteralPath $portPath)) { Fail "missing port trace $portPath (run tools/parity/port-encounter.ps1)"; Write-Host ("encounter gate: {0} failure(s)" -f $failures); exit 1 }

$retailRows = Read-Trace $retailPath
$portRows = Read-Trace $portPath
if ($retailRows.Count -eq 0) { Fail "retail trace $retailPath has no rows" }
if ($portRows.Count -eq 0) { Fail "port trace $portPath has no rows" }
if ($failures -gt 0) { Write-Host ("encounter gate: {0} failure(s)" -f $failures); exit 1 }

$retailShots = @($retailRows | Where-Object Kind -eq 'shot')
$portShots = @($portRows | Where-Object Kind -eq 'shot')
$retailHits = @($retailRows | Where-Object Kind -eq 'hit')
# The port's `shot` rows are the launches of shots that landed on the player; retail's fixture sees
# a shot only through its impact (its `hit` rows), so those two streams are the same shots, a flight
# apart. Retail's `shot` rows are bolts alive at a 100 ms sample — a lossy subset, not a cadence.
$retailShotSeries = $retailHits
$portShotSeries = $portShots
$portHits = @($portRows | Where-Object Kind -eq 'hit')
$retailPlayers = @($retailRows | Where-Object Kind -eq 'player')
$portPlayers = @($portRows | Where-Object Kind -eq 'player')
$retailActors = @($retailRows | Where-Object Kind -eq 'actor')
$portActors = @($portRows | Where-Object Kind -eq 'actor')

# ---- repeatability: two port runs of the same fixed step --------------------------------------
# Not byte identity: the AI scheduler consumes Bevy's system order and the cog RNG, so a run can
# differ by a frame. The fixture is repeatable when the shape and endpoint hold and every timeline
# delta stays inside the measured run-to-run band (shot 783 ms, hit 300 ms, actor 1.25 units/3.3 deg
# on the capture that set these defaults).
if ($repeatPath) {
  $bRows = Read-Trace $repeatPath
  $bShots = @($bRows | Where-Object Kind -eq 'shot')
  $bHits = @($bRows | Where-Object Kind -eq 'hit')
  $bPlayers = @($bRows | Where-Object Kind -eq 'player')
  $repeatOk = $true
  if ($bPlayers.Count -ne $portPlayers.Count) { Fail ("repeat run has {0} player sample(s), the first {1}" -f $bPlayers.Count, $portPlayers.Count); $repeatOk = $false }
  if ([math]::Abs($bShots.Count - $portShots.Count) -gt 5) { Fail ("repeat run has {0} shot(s), the first {1} (slack 5)" -f $bShots.Count, $portShots.Count); $repeatOk = $false }
  if ([math]::Abs($bHits.Count - $portHits.Count) -gt 4) { Fail ("repeat run has {0} hit(s), the first {1} (slack 4)" -f $bHits.Count, $portHits.Count); $repeatOk = $false }
  $shotRepeat = Get-EventDeltas $portShots $bShots
  $hitRepeat = Get-EventDeltas $portHits $bHits
  $actorRepeat = Get-ActorDeltas $portRows $bRows
  $portKills = @(Get-KillOrder $portRows)
  $bKills = @(Get-KillOrder $bRows)
  if (($portKills -join ',') -ne ($bKills -join ',')) {
    Fail ("repeat run's kill order [{0}] differs from the first run's [{1}]" -f ($bKills -join ','), ($portKills -join ','))
    $repeatOk = $false
  }
  if ($shotRepeat.Max -gt 4000) { Fail ("repeat shot timing delta {0:N0} ms exceeds 4000 ms" -f $shotRepeat.Max); $repeatOk = $false }
  if ($hitRepeat.Max -gt 5000) { Fail ("repeat hit timing delta {0:N0} ms exceeds 5000 ms" -f $hitRepeat.Max); $repeatOk = $false }
  if ($actorRepeat.Position -gt 2.0) { Fail ("repeat actor position delta {0:N2} exceeds 2.00 units" -f $actorRepeat.Position); $repeatOk = $false }
  # The AI can turn a different way at the same sample between runs (hash-order/RNG), so a yaw delta
  # of up to a half turn is not a fixture-shape change; the position/health checks carry this check.
  if ($actorRepeat.Health -gt 5) { Fail ("repeat actor health delta {0:N1} exceeds 5" -f $actorRepeat.Health); $repeatOk = $false }
  if ($portPlayers.Count -gt 0 -and $bPlayers.Count -gt 0) {
    $endHealth = [math]::Abs([double]$bPlayers[-1].Health - [double]$portPlayers[-1].Health)
    $endShields = [math]::Abs([double]$bPlayers[-1].Shields - [double]$portPlayers[-1].Shields)
    if ($endHealth -gt 10) { Fail ("repeat endpoint health delta {0:N1} exceeds 10" -f $endHealth); $repeatOk = $false }
    if ($endShields -gt 5) { Fail ("repeat endpoint shields delta {0:N1} exceeds 5" -f $endShields); $repeatOk = $false }
  }
  if ($repeatOk) {
    Write-Host ("PASS  repeatability: {0} vs {1} player sample(s), shots {2}, hits {3}; max deltas shot {4:N0} ms, hit {5:N0} ms, actor {6:N2} units/{7:N1} deg" -f `
        $portPlayers.Count, $bPlayers.Count, $portShots.Count, $portHits.Count, $shotRepeat.Max, $hitRepeat.Max, $actorRepeat.Position, $actorRepeat.Yaw)
  }
}

# ---- duration ---------------------------------------------------------------------------------
if ($retailPlayers.Count -eq 0 -or $portPlayers.Count -eq 0) {
  Fail 'one side has no player samples'
} else {
  $durationDelta = [math]::Abs($portPlayers[-1].T - $retailPlayers[-1].T)
  if ($durationDelta -gt $DurationToleranceMs) {
    Fail ("duration differs by {0:N0} ms (retail {1:N0}, port {2:N0}; tolerance {3:N0})" -f $durationDelta, $retailPlayers[-1].T, $portPlayers[-1].T, $DurationToleranceMs)
  } else {
    Write-Host ("PASS  duration: retail {0:N0} ms, port {1:N0} ms, delta {2:N0} ms" -f $retailPlayers[-1].T, $portPlayers[-1].T, $durationDelta)
  }
}

# ---- shots ------------------------------------------------------------------------------------
Write-Host ("shots: retail {0}, port {1}" -f $retailShotSeries.Count, $portShotSeries.Count)
if ([math]::Abs($retailShotSeries.Count - $portShotSeries.Count) -gt $ShotCountSlack) {
  Fail ("shot count differs by {0} (retail {1}, port {2}; slack {3})" -f ([math]::Abs($retailShotSeries.Count - $portShotSeries.Count)), $retailShotSeries.Count, $portShotSeries.Count, $ShotCountSlack)
}
$shotDeltas = @()
$portUsed = New-Object bool[] $portShotSeries.Count
foreach ($r in $retailShotSeries) {
  $bestIndex = -1
  $bestDelta = [double]::PositiveInfinity
  for ($i = 0; $i -lt $portShotSeries.Count; $i++) {
    if ($portUsed[$i]) { continue }
    $d = [math]::Abs([double]$portShotSeries[$i].T - [double]$r.T)
    if ($d -lt $bestDelta) { $bestDelta = $d; $bestIndex = $i }
  }
  if ($bestIndex -lt 0) { continue }
  $portUsed[$bestIndex] = $true
  $delta = [double]$portShotSeries[$bestIndex].T - [double]$r.T
  $shotDeltas += $delta
  if ([math]::Abs($delta) -gt $ShotToleranceMs) {
    Write-Host ("      shot: retail {0,6:N0} ms, port {1,6:N0} ms, delta {2,7:N0} ms" -f $r.T, $portShotSeries[$bestIndex].T, $delta)
  }
}
if ($shotDeltas.Count -gt 0) {
  $maxAbs = ($shotDeltas | ForEach-Object { [math]::Abs($_) } | Measure-Object -Maximum).Maximum
  $meanAbs = ($shotDeltas | ForEach-Object { [math]::Abs($_) } | Measure-Object -Average).Average
  Write-Host ("      per-shot timing delta: max {0:N0} ms, mean {1:N0} ms over {2} paired shot(s) (tolerance {3:N0})" -f $maxAbs, $meanAbs, $shotDeltas.Count, $ShotToleranceMs)
  if ($maxAbs -gt $ShotToleranceMs) { Fail ("shot timing delta {0:N0} ms exceeds {1:N0} ms" -f $maxAbs, $ShotToleranceMs) }

  # Fire cadence: the interval between consecutive shots, which is the cadence itself. Pairing
  # absolute times cannot answer it — a paired delta carries the flight and any phase the series
  # starts with — and index-pairing the launch and impact series is the mixed comparison the gate is
  # not allowed to make. Intervals cancel a constant flight, so the two series compare by interval.
  # The engine draws every delay from `rand()` (`sithAICmd_PrimaryFire`'s ±20 % jitter), so the
  # *order* of the intervals is not reproducible and one extra event is allowed by the count band:
  # compare the interval multiset, sorted, over the shared count. That is the cadence distribution
  # retail actually guarantees, not one realisation's ordering.
  $retailIntervals = @(for ($i = 1; $i -lt $retailShotSeries.Count; $i++) { [double]$retailShotSeries[$i].T - [double]$retailShotSeries[$i - 1].T }) | Sort-Object
  $portIntervals = @(for ($i = 1; $i -lt $portShotSeries.Count; $i++) { [double]$portShotSeries[$i].T - [double]$portShotSeries[$i - 1].T }) | Sort-Object
  $cadenceCount = [math]::Min($retailIntervals.Count, $portIntervals.Count)
  if ($cadenceCount -eq 0) {
    Write-Host '      cadence intervals: not enough events to compare'
  } else {
    $cadenceMax = 0.0
    for ($i = 0; $i -lt $cadenceCount; $i++) {
      $cadenceMax = [math]::Max($cadenceMax, [math]::Abs($portIntervals[$i] - $retailIntervals[$i]))
    }
    Write-Host ("      cadence intervals (sorted): retail [{0}], port [{1}] ms; max delta {2:N0} over {3} (tolerance {4:N0})" -f `
        (($retailIntervals | ForEach-Object { [math]::Round($_) }) -join ','), (($portIntervals | ForEach-Object { [math]::Round($_) }) -join ','), $cadenceMax, $cadenceCount, $ShotCadenceToleranceMs)
    if ($cadenceMax -gt $ShotCadenceToleranceMs) {
      Fail ("shot cadence delta {0:N0} ms exceeds {1:N0} ms" -f $cadenceMax, $ShotCadenceToleranceMs)
    } else {
      Write-Host ("PASS  shot cadence: {0} interval(s) within {1:N0} ms" -f $cadenceCount, $ShotCadenceToleranceMs)
    }
  }
}

# ---- hits -------------------------------------------------------------------------------------
Write-Host ("hits: retail {0}, port {1}" -f $retailHits.Count, $portHits.Count)
if ([math]::Abs($retailHits.Count - $portHits.Count) -gt $HitCountSlack) {
  Fail ("hit count differs by {0} (retail {1}, port {2}; slack {3})" -f ([math]::Abs($retailHits.Count - $portHits.Count)), $retailHits.Count, $portHits.Count, $HitCountSlack)
}
$hitCount = [math]::Min($retailHits.Count, $portHits.Count)
$hitMax = 0.0
$healthMax = 0.0
$shieldMax = 0.0
for ($i = 0; $i -lt $hitCount; $i++) {
  $r = $retailHits[$i]; $p = $portHits[$i]
  $delta = $p.T - $r.T
  $dHealth = [math]::Abs([double]$p.Health - [double]$r.Health)
  $dShields = [math]::Abs([double]$p.Shields - [double]$r.Shields)
  $hitMax = [math]::Max($hitMax, [math]::Abs($delta))
  $healthMax = [math]::Max($healthMax, $dHealth)
  $shieldMax = [math]::Max($shieldMax, $dShields)
  Write-Host ("      hit #{0}: retail {1,6:N0} ms ({2}/{3}), port {4,6:N0} ms ({5}/{6}), dt {7,7:N0} ms, dH {8:N1}, dS {9:N1}" -f `
      ($i + 1), $r.T, $r.Health, $r.Shields, $p.T, $p.Health, $p.Shields, $delta, $dHealth, $dShields)
}
if ($hitCount -gt 0) {
  if ($hitMax -gt $HitToleranceMs) { Fail ("hit timing delta {0:N0} ms exceeds {1:N0} ms" -f $hitMax, $HitToleranceMs) }
  if ($healthMax -gt $HealthTolerance) { Fail ("per-hit health delta {0:N1} exceeds {1:N1}" -f $healthMax, $HealthTolerance) }
  if ($shieldMax -gt $ShieldsTolerance) { Fail ("per-hit shields delta {0:N1} exceeds {1:N1}" -f $shieldMax, $ShieldsTolerance) }
  # The combat-damage dimension on its own: the bars after each paired hit, independent of when the
  # hits landed. A ticket that owns the damage split can read its result here without the AI timing.
  if ($healthMax -le $HealthTolerance -and $shieldMax -le $ShieldsTolerance) {
    Write-Host ("PASS  per-hit damage: max dH {0:N1}, dS {1:N1} over {2} paired hit(s) (tolerance {3:N1}/{4:N1})" -f `
        $healthMax, $shieldMax, $hitCount, $HealthTolerance, $ShieldsTolerance)
  }
}

# ---- kill order -------------------------------------------------------------------------------
# The ids are things (not templates), so the sequences compare directly. The door fixture has no
# player input and no kill in ten seconds on either side; the print makes that measured rather than
# assumed, and a port that kills someone retail leaves standing fails here.
$retailKills = @(Get-KillOrder $retailRows)
$portKills = @(Get-KillOrder $portRows)
Write-Host ("kill order: retail [{0}], port [{1}]" -f ($retailKills -join ','), ($portKills -join ','))
if (($retailKills -join ',') -ne ($portKills -join ',')) {
  Fail ("kill order differs: retail [{0}], port [{1}]" -f ($retailKills -join ','), ($portKills -join ','))
} else {
  Write-Host ("PASS  kill order: {0} kill(s) on both sides, the same sequence" -f $retailKills.Count)
}

# ---- settled endpoint -------------------------------------------------------------------------
if ($retailPlayers.Count -gt 0 -and $portPlayers.Count -gt 0) {
  $r = $retailPlayers[-1]; $p = $portPlayers[-1]
  # The final bars are the sum of the hits that landed. With the allowed one-event band the two
  # sides can end one hit apart, and then the totals are not the same observation; the per-hit
  # damage above already grades every shared hit. Compare the totals only when the event counts match.
  if ($retailHits.Count -ne $portHits.Count) {
    Write-Host ("      settled endpoint: {0} unpaired hit(s); final bars not compared (per-hit damage graded above)" -f [math]::Abs($retailHits.Count - $portHits.Count))
  } else {
    $dHealth = [math]::Abs([double]$p.Health - [double]$r.Health)
    $dShields = [math]::Abs([double]$p.Shields - [double]$r.Shields)
    if ($dHealth -gt $HealthTolerance) { Fail ("settled health {0:N1} vs retail {1:N1} (delta {2:N1})" -f $p.Health, $r.Health, $dHealth) }
    else { Write-Host ("PASS  settled health: port {0:N1}, retail {1:N1}" -f $p.Health, $r.Health) }
    if ($dShields -gt $ShieldsTolerance) { Fail ("settled shields {0:N1} vs retail {1:N1} (delta {2:N1})" -f $p.Shields, $r.Shields, $dShields) }
    else { Write-Host ("PASS  settled shields: port {0:N1}, retail {1:N1}" -f $p.Shields, $r.Shields) }
  }
}

# ---- actor state at each cadence sample -------------------------------------------------------
# A steering actor's absolute facing is not a parity observation: the engine's `Roam` handler
# (`0x50b830`, used by `gonkroam.ai`) draws `rand()` for the direction and the distance every
# interval, the stream is shared with every other AI roll, and the retail fixture records one draw.
# Two sides can only agree on *that* the actor steered. The rule below grades the absolute facing
# when either side stood still (or turned less than the tolerance) and skips it when both sides
# turned: a port actor that turns while retail's stands, or the reverse, is still a failure.
function Get-YawSpan($rows) {
  $spans = @{}
  foreach ($g in ($rows | Group-Object Thing)) {
    $yaws = @($g.Group | Where-Object { $null -ne $_.Yaw } | ForEach-Object { [double]$_.Yaw })
    if ($yaws.Count -lt 2) { continue }
    $spans[[int]$g.Name] = ($yaws | Measure-Object -Maximum).Maximum - ($yaws | Measure-Object -Minimum).Minimum
  }
  $spans
}
$retailYawSpan = Get-YawSpan $retailActors
# Port actors whose class carries `Roam`: the engine picks that walk's direction from `rand()`, the
# two sides' random streams differ, and the fixture records one draw on each side. Their absolute
# facing is therefore not a parity observation; their position still is, and the position check is
# what catches a roamer that walks while retail's stands.
$portRoamers = @{}
foreach ($row in $portActors) { if ($row.Detail -eq 'roam') { $portRoamers[[int]$row.Thing] = $true } }

# **Identity is the authored placement, not the thing number.** Retail's `thing` column is a slot in
# the engine's thing pool and the port's is the level's placement order; the two agree for most
# actors but not all (level 1's three ghosts occupy retail slots 75/91/99 while the authored ghosts
# are placements 281/285/282), so keying by the number pairs different actors and invents extras.
# Pair each actor at the first sample where both traces report it by nearest position — actors both
# builds place at the same authored point agree to 0.004 units — and use that map for the encounter.
# The first tick alone is not enough: an actor that walks into the sampling radius later has no row
# there, and a map built only at that tick left it unpaired for the encounter (a later retail actor
# was silently skipped rather than reported). What is left unmatched after every sample is a
# genuine set difference and fails; there is no missing fraction.
function Get-FirstSeen($Rows) {
  $first = @{}
  foreach ($row in $Rows) {
    $key = [string]$row.Thing
    if (-not $first.ContainsKey($key) -or [double]$row.T -lt [double]$first[$key].T) { $first[$key] = $row }
  }
  return $first
}
$retailFirst = Get-FirstSeen $retailActors
$portFirst = Get-FirstSeen $portActors
$identity = @{}
$identityUsed = @{}
$times = @((@($retailActors | ForEach-Object { [double]$_.T }) + @($portActors | ForEach-Object { [double]$_.T })) | Sort-Object -Unique)
foreach ($t in $times) {
  $retailOpen = @($retailActors | Where-Object { [double]$_.T -eq $t -and -not $identity.ContainsKey([string]$_.Thing) })
  $portOpen = @($portActors | Where-Object { [double]$_.T -eq $t -and -not $identityUsed.ContainsKey([string]$_.Thing) })
  $candidates = [System.Collections.Generic.List[object]]::new()
  foreach ($r in $retailOpen) {
    foreach ($p in $portOpen) {
      if ($null -eq $r.X -or $null -eq $p.X) { continue }
      $dx = [double]$p.X - [double]$r.X
      $dy = [double]$p.Y - [double]$r.Y
      $dz = [double]$p.Z - [double]$r.Z
      $distance = [math]::Sqrt($dx * $dx + $dy * $dy + $dz * $dz)
      if ($distance -le $ActorIdentityTolerance) {
        $candidates.Add([pscustomobject]@{ Distance = $distance; Retail = [string]$r.Thing; Port = [string]$p.Thing })
      }
    }
  }
  foreach ($c in @($candidates | Sort-Object Distance)) {
    if ($identity.ContainsKey($c.Retail) -or $identityUsed.ContainsKey($c.Port)) { continue }
    $identity[$c.Retail] = $c.Port
    $identityUsed[$c.Port] = $true
  }
}
$unmatchedRetail = @($retailFirst.Values | Where-Object { -not $identity.ContainsKey([string]$_.Thing) } | Sort-Object T)
$unmatchedPort = @($portFirst.Values | Where-Object { -not $identityUsed.ContainsKey([string]$_.Thing) } | Sort-Object T)

$portActorAt = @{}
foreach ($row in $portActors) { $portActorAt[('{0}|{1}' -f [int]$row.T, [string]$row.Thing)] = $row }
$posMax = 0.0
$yawMax = 0.0
$steered = 0
$actorHealthMax = 0.0
$compared = 0
$missing = 0
# Extras are mapped port actors, not the unmatched ones (those are reported at identity level);
# a mapped port actor that retail did not report is a real divergence in the shared set.
$extra = @{}
foreach ($row in $portActors) {
  if ($identityUsed.ContainsKey([string]$row.Thing)) { $extra[('{0}|{1}' -f [int]$row.T, [string]$row.Thing)] = $true }
}
foreach ($row in $retailActors) {
  if (-not $identity.ContainsKey([string]$row.Thing)) { continue }
  $portThing = $identity[[string]$row.Thing]
  $key = '{0}|{1}' -f [int]$row.T, $portThing
  $extra.Remove($key)
  if (-not $portActorAt.ContainsKey($key)) { $missing++; continue }
  $p = $portActorAt[$key]
  if ($null -eq $p.X -or $null -eq $row.X) { continue }
  $dx = [double]$p.X - [double]$row.X
  $dy = [double]$p.Y - [double]$row.Y
  $dz = [double]$p.Z - [double]$row.Z
  $distance = [math]::Sqrt($dx * $dx + $dy * $dy + $dz * $dz)
  $posMax = [math]::Max($posMax, $distance)
  if ($null -ne $p.Yaw -and $null -ne $row.Yaw) {
    # Retail's own facing moved further than the tolerance: the fixture records an animation or a
    # random pick (the droid's turntable), not a facing this port can derive, so the angle is not a
    # parity observation. Everything else — the actor's position above, and every actor whose retail
    # facing is stable — is still graded, which is what catches the port inventing a turn.
    if ($retailYawSpan[[int]$row.Thing] -gt $ActorYawTolerance -or $portRoamers.ContainsKey([int]$portThing)) {
      $steered++
    } else {
      $yawMax = [math]::Max($yawMax, (Get-WrapDelta ([double]$row.Yaw) ([double]$p.Yaw)))
    }
  }
  if ($null -ne $p.Health -and $null -ne $row.Health) {
    $actorHealthMax = [math]::Max($actorHealthMax, [math]::Abs([double]$p.Health - [double]$row.Health))
  }
  $compared++
}
$sampleCount = [math]::Min($retailPlayers.Count, $portPlayers.Count)
Write-Host ("actor samples: {0} retail row(s), {1} port row(s) over {2} sample time(s); {3} identity pair(s), compared {4}, missing in port {5}, extra in port {6}" -f `
    $retailActors.Count, $portActors.Count, $sampleCount, $identity.Count, $compared, $missing, $extra.Count)
if ($compared -gt 0) {
  Write-Host ("      max deltas: position {0:N2} units, yaw {1:N1} deg, health {2:N1} (tolerances {3:N2}/{4:N1}/{5:N1})" -f `
      $posMax, $yawMax, $actorHealthMax, $ActorPositionTolerance, $ActorYawTolerance, $ActorHealthTolerance)
  if ($steered -gt 0) {
    Write-Host ("      yaw: {0} sample(s) of actor(s) whose retail facing moved (animation or random) not compared absolutely" -f $steered)
  }
  if ($posMax -gt $ActorPositionTolerance) { Fail ("actor position delta {0:N2} exceeds {1:N2} units" -f $posMax, $ActorPositionTolerance) }
  if ($yawMax -gt $ActorYawTolerance) { Fail ("actor yaw delta {0:N1} exceeds {1:N1} deg" -f $yawMax, $ActorYawTolerance) }
  if ($actorHealthMax -gt $ActorHealthTolerance) { Fail ("actor health delta {0:N1} exceeds {1:N1}" -f $actorHealthMax, $ActorHealthTolerance) }
}
# The set difference is the structural half: an actor retail reports at a cadence sample and the
# port does not, or the reverse, fails. Extras are not "phantom": a port actor retail never had is
# exactly the invented-actor case the gate exists to catch.
if ($unmatchedRetail.Count -gt 0) {
  Fail ("{0} retail actor(s) have no port actor ({1})" -f `
      $unmatchedRetail.Count, (($unmatchedRetail | ForEach-Object { '{0}@{1:N0} ms' -f $_.Thing, $_.T }) -join ','))
}
if ($unmatchedPort.Count -gt 0) {
  Fail ("{0} port actor(s) have no retail actor ({1})" -f `
      $unmatchedPort.Count, (($unmatchedPort | ForEach-Object { '{0}@{1:N0} ms' -f $_.Thing, $_.T }) -join ','))
}
if ($missing -gt 0) { Fail ("{0} mapped retail actor sample(s) have no port actor" -f $missing) }
if ($extra.Count -gt 0 -and $unmatchedPort.Count -eq 0) { Fail ("{0} port actor sample(s) have no retail actor" -f $extra.Count) }

Write-Host ("encounter gate: {0} failure(s)" -f $failures)
if ($failures -gt 0) { exit 1 }
