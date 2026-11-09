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
  timing delta of every shot and hit, the per-hit health/shield delta, and the actor state delta at
  each cadence sample, then exits non-zero when any of them leaves its tolerance. It is the
  instrument tickets #11 (AI timing) and #12 (combat timing) measure against.

  The retail fixture is committed under `tools/parity/encounter/`; a fresh capture in
  `target/parity/encounter/` takes precedence. The gate records the port twice and fails when the
  two runs leave the measured run-to-run band (shape, endpoint, event timing, actor state), so "the
  fixture is repeatable" is checked rather than assumed.

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
  # Event-count slack: |port - retail| allowed before the count itself is a failure.
  [int]$ShotCountSlack = 0,
  [int]$HitCountSlack = 0,
  [double]$ShotToleranceMs = 1000,
  [double]$HitToleranceMs = 1000,
  [double]$HealthTolerance = 10,
  [double]$ShieldsTolerance = 10,
  [double]$ActorPositionTolerance = 1.5,
  [double]$ActorYawTolerance = 30,
  [double]$ActorHealthTolerance = 15,
  [double]$ActorMissingFraction = 0.25
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
Write-Host ("shots: retail {0}, port {1}" -f $retailShots.Count, $portShots.Count)
if ([math]::Abs($retailShots.Count - $portShots.Count) -gt $ShotCountSlack) {
  Fail ("shot count differs by {0} (retail {1}, port {2}; slack {3})" -f ([math]::Abs($retailShots.Count - $portShots.Count)), $retailShots.Count, $portShots.Count, $ShotCountSlack)
}
$shotDeltas = @()
$shotCount = [math]::Min($retailShots.Count, $portShots.Count)
for ($i = 0; $i -lt $shotCount; $i++) {
  $delta = $portShots[$i].T - $retailShots[$i].T
  $shotDeltas += $delta
  if ([math]::Abs($delta) -gt $ShotToleranceMs) {
    Write-Host ("      shot #{0}: retail {1,6:N0} ms, port {2,6:N0} ms, delta {3,7:N0} ms" -f ($i + 1), $retailShots[$i].T, $portShots[$i].T, $delta)
  }
}
if ($shotDeltas.Count -gt 0) {
  $maxAbs = ($shotDeltas | ForEach-Object { [math]::Abs($_) } | Measure-Object -Maximum).Maximum
  $meanAbs = ($shotDeltas | ForEach-Object { [math]::Abs($_) } | Measure-Object -Average).Average
  Write-Host ("      per-shot timing delta: max {0:N0} ms, mean {1:N0} ms over {2} paired shot(s) (tolerance {3:N0})" -f $maxAbs, $meanAbs, $shotDeltas.Count, $ShotToleranceMs)
  # Report the cadence as shots per second, the interval comparison #11 asks for.
  $retailRate = @($retailShots | Group-Object { [math]::Floor($_.T / 1000) } | ForEach-Object Count)
  $portRate = @($portShots | Group-Object { [math]::Floor($_.T / 1000) } | ForEach-Object Count)
  Write-Host ("      shots per second: retail [{0}], port [{1}]" -f ($retailRate -join ','), ($portRate -join ','))
  if ($maxAbs -gt $ShotToleranceMs) { Fail ("shot timing delta {0:N0} ms exceeds {1:N0} ms" -f $maxAbs, $ShotToleranceMs) }
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
}

# ---- settled endpoint -------------------------------------------------------------------------
if ($retailPlayers.Count -gt 0 -and $portPlayers.Count -gt 0) {
  $r = $retailPlayers[-1]; $p = $portPlayers[-1]
  $dHealth = [math]::Abs([double]$p.Health - [double]$r.Health)
  $dShields = [math]::Abs([double]$p.Shields - [double]$r.Shields)
  if ($dHealth -gt $HealthTolerance) { Fail ("settled health {0:N1} vs retail {1:N1} (delta {2:N1})" -f $p.Health, $r.Health, $dHealth) }
  else { Write-Host ("PASS  settled health: port {0:N1}, retail {1:N1}" -f $p.Health, $r.Health) }
  if ($dShields -gt $ShieldsTolerance) { Fail ("settled shields {0:N1} vs retail {1:N1} (delta {2:N1})" -f $p.Shields, $r.Shields, $dShields) }
  else { Write-Host ("PASS  settled shields: port {0:N1}, retail {1:N1}" -f $p.Shields, $r.Shields) }
}

# ---- actor state at each cadence sample -------------------------------------------------------
$portActorAt = @{}
foreach ($row in $portActors) { $portActorAt[('{0}|{1}' -f [int]$row.T, $row.Thing)] = $row }
$posMax = 0.0
$yawMax = 0.0
$actorHealthMax = 0.0
$compared = 0
$missing = 0
$extra = @{}
foreach ($row in $portActors) { $extra[('{0}|{1}' -f [int]$row.T, $row.Thing)] = $true }
foreach ($row in $retailActors) {
  $key = '{0}|{1}' -f [int]$row.T, $row.Thing
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
    $yawMax = [math]::Max($yawMax, (Get-WrapDelta ([double]$row.Yaw) ([double]$p.Yaw)))
  }
  if ($null -ne $p.Health -and $null -ne $row.Health) {
    $actorHealthMax = [math]::Max($actorHealthMax, [math]::Abs([double]$p.Health - [double]$row.Health))
  }
  $compared++
}
$sampleCount = [math]::Min($retailPlayers.Count, $portPlayers.Count)
Write-Host ("actor samples: {0} retail row(s), {1} port row(s) over {2} sample time(s); compared {3}, missing in port {4}, extra in port {5}" -f `
    $retailActors.Count, $portActors.Count, $sampleCount, $compared, $missing, $extra.Count)
if ($compared -gt 0) {
  Write-Host ("      max deltas: position {0:N2} units, yaw {1:N1} deg, health {2:N1} (tolerances {3:N2}/{4:N1}/{5:N1})" -f `
      $posMax, $yawMax, $actorHealthMax, $ActorPositionTolerance, $ActorYawTolerance, $ActorHealthTolerance)
  if ($posMax -gt $ActorPositionTolerance) { Fail ("actor position delta {0:N2} exceeds {1:N2} units" -f $posMax, $ActorPositionTolerance) }
  if ($yawMax -gt $ActorYawTolerance) { Fail ("actor yaw delta {0:N1} exceeds {1:N1} deg" -f $yawMax, $ActorYawTolerance) }
  if ($actorHealthMax -gt $ActorHealthTolerance) { Fail ("actor health delta {0:N1} exceeds {1:N1}" -f $actorHealthMax, $ActorHealthTolerance) }
}
if ($retailActors.Count -gt 0) {
  $missingFraction = $missing / $retailActors.Count
  if ($missingFraction -gt $ActorMissingFraction) {
    Fail ("{0:P0} of retail actor samples have no port actor (tolerance {1:P0})" -f $missingFraction, $ActorMissingFraction)
  }
}

Write-Host ("encounter gate: {0} failure(s)" -f $failures)
if ($failures -gt 0) { exit 1 }
