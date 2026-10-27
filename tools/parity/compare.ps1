#requires -Version 7
<#
.SYNOPSIS
  Retail-vs-port parity comparison: side-by-side images, contact sheet, report.md.

.DESCRIPTION
  Pairs target/parity/retail/<name>.png with target/parity/port/<name>.png for
  every row of tools/parity/shots.tsv (rows missing a side are reported as
  "not captured yet").

  For each pair it:
    - reads the PNG sizes with .NET (no extra tools beyond ffmpeg),
    - detects and crops the retail black letterbox with ffmpeg bbox,
    - computes SSIM and mean absolute difference on a common grayscale canvas,
    - writes target/parity/compare/<name>-side.png (retail world | port),
  then writes target/parity/compare/contact.png and report.md.

  Retail and port are different renderers with different framing: the numbers
  are a triage signal, not a pixel-equality claim.

.EXAMPLE
  pwsh -File tools/parity/compare.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$Root        = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)   # tools\parity -> repo root
$ShotsPath   = Join-Path $Root 'tools\parity\shots.tsv'
$RetailDir   = Join-Path $Root 'target\parity\retail'
$PortDir     = Join-Path $Root 'target\parity\port'
$CompareDir  = Join-Path $Root 'target\parity\compare'
$ReportPath  = Join-Path $CompareDir 'report.md'
$ContactPath = Join-Path $CompareDir 'contact.png'

# Metric normalization: each frame is stretched onto one common grayscale canvas.
$NormalizedWidth  = 640
$NormalizedHeight = 360
$SideHeight       = 480
$ContactWidth     = 960
# The retail pal8 letterbox bars are ~16 luma; bbox treats <= this value as border.
# (This is also ffmpeg cropdetect's default limit=24.)
$BorderThreshold  = 24

$Ffmpeg = $null

function Get-ShotRows {
    $rows = @()
    foreach ($line in Get-Content -LiteralPath $ShotsPath) {
        if ($line -match '^\s*(#|$)') { continue }
        $f = $line -split "`t"
        if ($f.Count -lt 7 -or $f[0].Trim() -eq 'name') { continue }
        $rows += [pscustomobject]@{
            Name  = $f[0].Trim()
            Level = $f[1].Trim()
            X     = $f[2].Trim()
            Y     = $f[3].Trim()
            Z     = $f[4].Trim()
            Pitch = $f[5].Trim()
            Yaw   = $f[6].Trim()
        }
    }
    return $rows
}

# PNG size straight from the IHDR chunk, so ffprobe is not needed.
function Get-PngSize {
    param([string]$Path)
    $fs = [System.IO.File]::OpenRead($Path)
    try {
        $buf = [byte[]]::new(24)
        $read = $fs.Read($buf, 0, 24)
        if ($read -lt 24) { throw "short PNG header: $Path" }
        $signature = [byte[]](0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
        for ($i = 0; $i -lt 8; $i++) {
            if ($buf[$i] -ne $signature[$i]) { throw "not a PNG: $Path" }
        }
        return [pscustomobject]@{
            Width  = [System.Net.IPAddress]::NetworkToHostOrder([BitConverter]::ToInt32($buf, 16))
            Height = [System.Net.IPAddress]::NetworkToHostOrder([BitConverter]::ToInt32($buf, 20))
        }
    } finally { $fs.Dispose() }
}

function Invoke-Ffmpeg {
    param([string[]]$Arguments)
    $errFile = [System.IO.Path]::GetTempFileName()
    try {
        $stdout = & $Ffmpeg @Arguments 2> $errFile
        $code = $LASTEXITCODE
        $stdoutText = if ($null -eq $stdout) { '' } else { ($stdout | Out-String) }
        $stderrText = if (Test-Path -LiteralPath $errFile) { [System.IO.File]::ReadAllText($errFile) } else { '' }
        return [pscustomobject]@{ ExitCode = $code; Stdout = $stdoutText; Stderr = $stderrText }
    } finally {
        Remove-Item -LiteralPath $errFile -Force -ErrorAction SilentlyContinue
    }
}

function Test-CropSane {
    param($Crop, [int]$Width, [int]$Height)
    if ($null -eq $Crop) { return $false }
    if ($Crop.Width -le 0 -or $Crop.Height -le 0) { return $false }
    if ($Crop.X -lt 0 -or $Crop.Y -lt 0) { return $false }
    if (($Crop.X + $Crop.Width) -gt $Width) { return $false }
    if (($Crop.Y + $Crop.Height) -gt $Height) { return $false }
    return $true
}

# One ffmpeg bbox pass over an explicit filter chain; returns W/H/X/Y or $null.
function Get-BboxCrop {
    param([string]$Path, [string]$FilterChain)
    $res = Invoke-Ffmpeg @('-hide_banner', '-loglevel', 'info', '-i', $Path,
        '-vf', $FilterChain, '-f', 'null', '-')
    $found = [regex]::Matches($res.Stderr, 'crop=(\d+):(\d+):(\d+):(\d+)')
    if ($found.Count -eq 0) { return $null }
    $g = $found[$found.Count - 1].Groups
    return [pscustomobject]@{
        Width  = [int]$g[1].Value
        Height = [int]$g[2].Value
        X      = [int]$g[3].Value
        Y      = [int]$g[4].Value
    }
}

# Parse the retail coords sidecar's `world_crop`. It is expressed in RAW-frame pixels:
# accepts {x,y,w,h} / {width,height,x,y}, "WxH+X+Y", or a 4-element array tried as
# [x,y,w,h] then [w,h,x,y] (whichever satisfies the raw frame bounds).
function ConvertTo-WorldCrop {
    param($Value, [int]$Width, [int]$Height)
    $candidates = @()
    if ($Value -is [string]) {
        if ($Value -match '^\s*(\d+)x(\d+)\+(\d+)\+(\d+)\s*$') {
            $candidates += [pscustomobject]@{
                Width = [int]$Matches[1]; Height = [int]$Matches[2]; X = [int]$Matches[3]; Y = [int]$Matches[4]
            }
        }
    } elseif ($Value -is [System.Array]) {
        $v = @($Value | ForEach-Object { [int]$_ })
        if ($v.Count -eq 4) {
            $candidates += [pscustomobject]@{ Width = $v[2]; Height = $v[3]; X = $v[0]; Y = $v[1] }   # [x,y,w,h]
            $candidates += [pscustomobject]@{ Width = $v[0]; Height = $v[1]; X = $v[2]; Y = $v[3] }   # [w,h,x,y]
        }
    } else {
        $w = $Value.width
        $h = $Value.height
        if ($null -ne $w -and $null -ne $h) {
            $x = if ($null -ne $Value.x) { [int]$Value.x } else { 0 }
            $y = if ($null -ne $Value.y) { [int]$Value.y } else { 0 }
            $candidates += [pscustomobject]@{ Width = [int]$w; Height = [int]$h; X = $x; Y = $y }
        }
    }
    foreach ($c in $candidates) {
        if (Test-CropSane -Crop $c -Width $Width -Height $Height) { return $c }
    }
    return $null
}

# Retail coords sidecar: world_crop in raw-frame coordinates plus the raw/png sizes.
function Get-SidecarInfo {
    param([string]$RetailPng, [int]$Width, [int]$Height)
    $json = [System.IO.Path]::ChangeExtension($RetailPng, '.json')
    if (-not (Test-Path -LiteralPath $json)) { return $null }
    try { $j = Get-Content -LiteralPath $json -Raw | ConvertFrom-Json } catch { return $null }
    $rawW = if ($null -ne $j.raw_width) { [int]$j.raw_width } else { $null }
    $rawH = if ($null -ne $j.raw_height) { [int]$j.raw_height } else { $null }
    $refW = if ($null -ne $rawW) { $rawW } else { $Width }
    $refH = if ($null -ne $rawH) { $rawH } else { $Height }
    $wc = $null
    if ($null -ne $j.world_crop) {
        $wc = ConvertTo-WorldCrop -Value $j.world_crop -Width $refW -Height $refH
    }
    return [pscustomobject]@{ WorldCrop = $wc; RawWidth = $rawW; RawHeight = $rawH }
}

# Detect the retail world box without being fooled by the -devMode dev-log text
# boxes in the top corners: bbox runs on a middle horizontal band (which excludes
# them), then the detected x-range is applied to the full image height.
# Returns an object with Source/Detected/NoBars/WorldBox/AlreadyCropped describing
# which rule was used; Width/Height/X/Y is the crop to apply to the current PNG.
function Get-RetailCrop {
    param([string]$Path, [int]$Width, [int]$Height)

    $sidecar = Get-SidecarInfo -RetailPng $Path -Width $Width -Height $Height
    $wc = if ($null -ne $sidecar) { $sidecar.WorldCrop } else { $null }
    if ($null -ne $wc) {
        $worldBox = "$($wc.Width)x$($wc.Height)+$($wc.X)+$($wc.Y)"
        $rawSize = if ($null -ne $sidecar.RawWidth -and $null -ne $sidecar.RawHeight) {
            "$($sidecar.RawWidth)x$($sidecar.RawHeight)"
        } else { 'raw' }
        # The PNG is the raw frame: world_crop is a crop inside it.
        if (Test-CropSane -Crop $wc -Width $Width -Height $Height) {
            $noBars = ($wc.X -eq 0 -and $wc.Y -eq 0 -and $wc.Width -eq $Width -and $wc.Height -eq $Height)
            return [pscustomobject]@{
                Width = $wc.Width; Height = $wc.Height; X = $wc.X; Y = $wc.Y
                Source = "sidecar JSON world_crop ($worldBox of $rawSize raw frame)"
                Detected = (-not $noBars); NoBars = $noBars
                WorldBox = $worldBox; AlreadyCropped = $false
            }
        }
        # The PNG is already cropped to the world box (its size matches world_crop while
        # world_crop is in raw-frame coordinates): use it as-is.
        if ($wc.Width -eq $Width -and $wc.Height -eq $Height) {
            return [pscustomobject]@{
                Width = $Width; Height = $Height; X = 0; Y = 0
                Source = "sidecar JSON world_crop (retail PNG already cropped: $worldBox of $rawSize raw frame)"
                Detected = $false; NoBars = $true
                WorldBox = $worldBox; AlreadyCropped = $true
            }
        }
        # Otherwise the sidecar crop is not usable for this image; fall back to bbox.
    }

    # Middle horizontal band: skips the top-corner dev text entirely.
    $bandHeight = [Math]::Min(300, $Height)
    $bandY = [Math]::Min(300, $Height - $bandHeight)
    $band = Get-BboxCrop -Path $Path -FilterChain "crop=iw:${bandHeight}:0:${bandY},bbox=min_val=$BorderThreshold"
    if ($null -eq $band -or $band.Width -le 0 -or $band.Width -ge $Width) {
        # No vertical bars (or detection failed): leave the frame uncropped.
        return [pscustomobject]@{
            Width = $Width; Height = $Height; X = 0; Y = 0
            Source = 'none (full frame: no bars detected)'; Detected = $false; NoBars = $true
            WorldBox = "$($Width)x$($Height)+0+0"; AlreadyCropped = $false
        }
    }

    $crop = [pscustomobject]@{
        Width = $band.Width; Height = $Height; X = $band.X; Y = 0
        Source = "ffmpeg bbox band (y=${bandY}..$($bandY + $bandHeight - 1); top corners excluded)"
        Detected = $true; NoBars = $false
        WorldBox = ''; AlreadyCropped = $false
    }

    # Refine the y-range inside the detected world columns. Accept it only when it
    # spans essentially the full height: retail letterboxes the world horizontally.
    $vertical = Get-BboxCrop -Path $Path -FilterChain "crop=$($crop.Width):ih:$($crop.X):0,bbox=min_val=$BorderThreshold"
    if ($null -ne $vertical -and $vertical.Y -le 16 -and
        ($vertical.Y + $vertical.Height) -ge ($Height - 16)) {
        $crop.Height = $vertical.Height
        $crop.Y = $vertical.Y
    }
    $crop.WorldBox = "$($crop.Width)x$($crop.Height)+$($crop.X)+$($crop.Y)"
    return $crop
}

function Measure-Ssim {
    param([string]$Retail, $Crop, [string]$Port)
    $chain = "[0:v]crop=$($Crop.Width):$($Crop.Height):$($Crop.X):$($Crop.Y)," +
        "scale=${NormalizedWidth}:${NormalizedHeight}:flags=bilinear,setsar=1,format=gray[a];" +
        "[1:v]scale=${NormalizedWidth}:${NormalizedHeight}:flags=bilinear,setsar=1,format=gray[b];[a][b]ssim"
    $res = Invoke-Ffmpeg @('-hide_banner', '-loglevel', 'info', '-i', $Retail, '-i', $Port,
        '-filter_complex', $chain, '-f', 'null', '-')
    if ($res.ExitCode -ne 0) { throw "ffmpeg ssim failed: $($res.Stderr)" }
    $m = [regex]::Match($res.Stderr, 'SSIM Y:([0-9.]+) \(([^)]+)\) All:([0-9.]+)')
    if (-not $m.Success) { throw "ffmpeg reported no SSIM line: $($res.Stderr)" }
    return [pscustomobject]@{
        All = [double]$m.Groups[3].Value
        Db  = $m.Groups[2].Value.Trim()
    }
}

function Measure-Mad {
    param([string]$Retail, $Crop, [string]$Port)
    $chain = "[0:v]crop=$($Crop.Width):$($Crop.Height):$($Crop.X):$($Crop.Y)," +
        "scale=${NormalizedWidth}:${NormalizedHeight}:flags=bilinear,setsar=1,format=gray[a];" +
        "[1:v]scale=${NormalizedWidth}:${NormalizedHeight}:flags=bilinear,setsar=1,format=gray[b];" +
        "[a][b]blend=all_mode=difference,signalstats,metadata=print:file=-[d]"
    $res = Invoke-Ffmpeg @('-hide_banner', '-loglevel', 'info', '-i', $Retail, '-i', $Port,
        '-filter_complex', $chain, '-map', '[d]', '-f', 'null', '-')
    if ($res.ExitCode -ne 0) { throw "ffmpeg difference failed: $($res.Stderr)" }
    $m = [regex]::Match($res.Stdout, 'lavfi\.signalstats\.YAVG=([0-9.]+)')
    if (-not $m.Success) { throw "ffmpeg reported no mean difference: $($res.Stdout) $($res.Stderr)" }
    return [double]$m.Groups[1].Value
}

function New-SideBySide {
    param([string]$Retail, $Crop, [string]$Port, [string]$Out)
    $chain = "[0:v]crop=$($Crop.Width):$($Crop.Height):$($Crop.X):$($Crop.Y),setsar=1," +
        "scale=-2:${SideHeight},pad=iw+4:ih:0:0:color=0x505050[r];" +
        "[1:v]setsar=1,scale=-2:${SideHeight}[p];[r][p]hstack=inputs=2"
    $res = Invoke-Ffmpeg @('-hide_banner', '-loglevel', 'error', '-y', '-i', $Retail, '-i', $Port,
        '-filter_complex', $chain, '-frames:v', '1', $Out)
    if ($res.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $Out)) {
        throw "ffmpeg side-by-side failed (exit $($res.ExitCode)): $($res.Stderr)"
    }
}

function New-ContactSheet {
    param([string[]]$Sides, [string]$Out)
    if ($Sides.Count -eq 1) {
        $res = Invoke-Ffmpeg @('-hide_banner', '-loglevel', 'error', '-y', '-i', $Sides[0],
            '-vf', "scale=${ContactWidth}:-2", '-frames:v', '1', $Out)
    } else {
        $ff = @('-hide_banner', '-loglevel', 'error', '-y')
        foreach ($side in $Sides) { $ff += @('-i', $side) }
        $chains = @()
        $labels = ''
        for ($i = 0; $i -lt $Sides.Count; $i++) {
            $chains += "[${i}:v]scale=${ContactWidth}:-2[s${i}]"
            $labels += "[s${i}]"
        }
        $chains += "${labels}vstack=inputs=$($Sides.Count)"
        $ff += @('-filter_complex', ($chains -join ';'), '-frames:v', '1', $Out)
        $res = Invoke-Ffmpeg $ff
    }
    if ($res.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $Out)) {
        throw "ffmpeg contact sheet failed (exit $($res.ExitCode)): $($res.Stderr)"
    }
}

function Format-Pose {
    param($Row)
    if ($Row.X -eq '' -or $Row.X -eq '-') {
        return 'level spawn (no --shot-at; port run uses plain --start)'
    }
    $note = ''
    if ($Row.Pitch -ne '-' -and [double]$Row.Pitch -ne 0) {
        $note = " (pitch $($Row.Pitch) unsupported: port --shot-at is horizontal-only)"
    }
    return "x=$($Row.X) y=$($Row.Y) z=$($Row.Z) yaw=$($Row.Yaw)$note"
}

# Best-effort read of a retail coords sidecar written by the retail-capture side.
function Get-RetailMeta {
    param([string]$RetailPng)
    $json = [System.IO.Path]::ChangeExtension($RetailPng, '.json')
    if (-not (Test-Path -LiteralPath $json)) { return $null }
    try {
        $j = Get-Content -LiteralPath $json -Raw | ConvertFrom-Json
        if ($null -eq $j.pos -or $null -eq $j.pyr) { return $null }
        $pos = (@($j.pos) | ForEach-Object { '{0:F2}' -f [double]$_ }) -join ', '
        $pyr = (@($j.pyr) | ForEach-Object { '{0:F2}' -f [double]$_ }) -join ', '
        return "Pos ($pos), PYR ($pyr), sector $($j.sector)"
    } catch { return $null }
}

# ---------------------------------------------------------------- setup

if (-not (Test-Path -LiteralPath $ShotsPath)) { throw "missing shots list: $ShotsPath" }
$ffmpegCmd = Get-Command ffmpeg -ErrorAction SilentlyContinue
if (-not $ffmpegCmd) { throw 'ffmpeg not found on PATH (needed for crop detection and metrics)' }
$Ffmpeg = $ffmpegCmd.Source
New-Item -ItemType Directory -Force -Path $CompareDir | Out-Null

# ---------------------------------------------------------------- compare

$rows = @(Get-ShotRows)
$pairs = @()
$missing = @()
$errors = @()

foreach ($row in $rows) {
    $retailPath = Join-Path $RetailDir "$($row.Name).png"
    $portPath   = Join-Path $PortDir "$($row.Name).png"
    $haveRetail = Test-Path -LiteralPath $retailPath
    $havePort   = Test-Path -LiteralPath $portPath
    if (-not ($haveRetail -and $havePort)) {
        $missing += [pscustomobject]@{ Name = $row.Name; Retail = $haveRetail; Port = $havePort }
        continue
    }
    try {
        $retailInfo = Get-PngSize $retailPath
        $portInfo   = Get-PngSize $portPath
        $crop = Get-RetailCrop -Path $retailPath -Width $retailInfo.Width -Height $retailInfo.Height
        $borders = [pscustomobject]@{
            Left   = $crop.X
            Top    = $crop.Y
            Right  = $retailInfo.Width - ($crop.X + $crop.Width)
            Bottom = $retailInfo.Height - ($crop.Y + $crop.Height)
        }
        $ssim = Measure-Ssim -Retail $retailPath -Crop $crop -Port $portPath
        $mad  = Measure-Mad  -Retail $retailPath -Crop $crop -Port $portPath
        $sidePath = Join-Path $CompareDir "$($row.Name)-side.png"
        New-SideBySide -Retail $retailPath -Crop $crop -Port $portPath -Out $sidePath

        $pairs += [pscustomobject]@{
            Row        = $row
            RetailInfo = $retailInfo
            PortInfo   = $portInfo
            Crop       = $crop
            Borders    = $borders
            Ssim       = $ssim
            Mad        = $mad
            Side       = $sidePath
            Pose       = (Format-Pose $row)
            RetailMeta = (Get-RetailMeta $retailPath)
        }
        Write-Host ("{0}: SSIM {1:F4} ({2} dB), mean abs diff {3:F1}/255" -f $row.Name, $ssim.All, $ssim.Db, $mad)
    } catch {
        $errors += [pscustomobject]@{ Name = $row.Name; Message = $_.Exception.Message }
        Write-Host ("{0}: FAILED - {1}" -f $row.Name, $_.Exception.Message)
    }
}

# Worst first: lowest SSIM = most different.
$ordered = @($pairs | Sort-Object { $_.Ssim.All })

if ($ordered.Count -gt 0) {
    New-ContactSheet -Sides @($ordered | ForEach-Object { $_.Side }) -Out $ContactPath
} else {
    Remove-Item -LiteralPath $ContactPath -Force -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------- report

$lines = [System.Collections.Generic.List[string]]::new()
$generated = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
$lines.Add('# Parity comparison: retail vs port')
$lines.Add('')
$lines.Add("Generated: $generated")
$lines.Add('Shot list: `tools/parity/shots.tsv`')
$lines.Add("Pairs compared: $($ordered.Count); not captured yet: $($missing.Count); failed: $($errors.Count)")
$lines.Add('')
$lines.Add('**Triage, not equality.** Retail and port use different renderers and framing;')
$lines.Add('SSIM and mean absolute difference are direction-finding signals, not a claim that')
$lines.Add('the two images should match pixel for pixel. See "Framing and method" below.')
$lines.Add('')

if ($ordered.Count -eq 0) {
    $lines.Add('No complete pairs yet: need both `target/parity/retail/<name>.png` and')
    $lines.Add('`target/parity/port/<name>.png` for a row of `tools/parity/shots.tsv`.')
    $lines.Add('')
} else {
    $lines.Add('## Summary (worst first)')
    $lines.Add('')
    $lines.Add('| # | shot | level | SSIM (All) | SSIM (dB) | mean abs diff | retail world crop | port frame |')
    $lines.Add('|--:|------|-------|-----------:|----------:|--------------:|-------------------|------------|')
    $n = 0
    foreach ($p in $ordered) {
        $n++
        $cropText = if ($p.Crop.WorldBox) { $p.Crop.WorldBox } else { "$($p.Crop.Width)x$($p.Crop.Height)+$($p.Crop.X)+$($p.Crop.Y)" }
        $portText = "$($p.PortInfo.Width)x$($p.PortInfo.Height)"
        $lines.Add(("| {0} | {1} | {2} | {3:F4} | {4} | {5:F1} | {6} | {7} |" -f
            $n, $p.Row.Name, $p.Row.Level, $p.Ssim.All, $p.Ssim.Db, $p.Mad, $cropText, $portText))
    }
    $lines.Add('')
    $lines.Add('Contact sheet (worst first, top to bottom): `target/parity/compare/contact.png`')
    $lines.Add('')

    foreach ($p in $ordered) {
        $name = $p.Row.Name
        $retailRel = "target/parity/retail/$name.png"
        $portRel   = "target/parity/port/$name.png"
        $sideRel   = "target/parity/compare/$name-side.png"
        $lines.Add("## $name")
        $lines.Add('')
        $lines.Add('- pose: ' + $p.Pose)
        if ($p.RetailMeta) { $lines.Add('- retail coords metadata: ' + $p.RetailMeta) }
        if ($p.Crop.AlreadyCropped) {
            $lines.Add('- retail: `' + $retailRel + '` ' + "$($p.RetailInfo.Width)x$($p.RetailInfo.Height)" +
                ' pal8; the PNG is already cropped to the world box (sidecar world_crop ' +
                $p.Crop.WorldBox + '); used as-is, no further crop')
        } elseif ($p.Crop.NoBars) {
            $lines.Add('- retail: `' + $retailRel + '` ' + "$($p.RetailInfo.Width)x$($p.RetailInfo.Height)" +
                ' pal8; no black border detected, full frame used')
        } else {
            $b = $p.Borders
            $lines.Add('- retail: `' + $retailRel + '` ' + "$($p.RetailInfo.Width)x$($p.RetailInfo.Height)" +
                ' pal8; detected world crop **' + "$($p.Crop.Width)x$($p.Crop.Height)+$($p.Crop.X)+$($p.Crop.Y)" +
                "** (border removed left $($b.Left), right $($b.Right), top $($b.Top), bottom $($b.Bottom) px)")
        }
        $lines.Add('- crop source: ' + $p.Crop.Source)
        $lines.Add('- port: `' + $portRel + '` ' + "$($p.PortInfo.Width)x$($p.PortInfo.Height)" +
            ' rgb24; the port HUD is drawn into the image (retail software captures have no HUD)')
        $lines.Add('- side-by-side (left: retail world, right: port): `' + $sideRel + '`')
        $lines.Add(("- SSIM (All, grayscale ${NormalizedWidth}x${NormalizedHeight}): **{0:F4}** ({1} dB)" -f $p.Ssim.All, $p.Ssim.Db))
        $lines.Add(("- mean absolute difference (grayscale ${NormalizedWidth}x${NormalizedHeight}): **{0:F1} / 255**" -f $p.Mad))
        $lines.Add('')
    }
}

$lines.Add('## Framing and method')
$lines.Add('')
$lines.Add('- Retail side: the GOG game captured with its 8-bit software renderer; `JSHOT*.PCX`')
$lines.Add('  converted to PNG: 1024x768 pal8. `viewSize=5` letterboxes the world view inside that')
$lines.Add('  buffer (near-black border, about 16 luma). A full-frame `ffmpeg bbox` is fooled by')
$lines.Add('  the `-devMode` dev-log text boxes in the top corners, so the world box is detected')
$lines.Add('  on a middle horizontal band that excludes them (`crop=iw:<band>:0:<y>`, bbox')
$lines.Add('  threshold ' + $BorderThreshold + ', the cropdetect default limit); the detected x-range is then')
$lines.Add('  applied to the full image height, and y is refined only when it spans essentially')
$lines.Add('  the full height. If the retail sidecar JSON has a `world_crop` field it is used')
$lines.Add('  instead: applied inside the PNG when the PNG is the raw frame, or treated as')
$lines.Add('  already applied when the PNG size matches world_crop (the PNG is then the world')
$lines.Add('  box). If the box equals the full frame the image is treated as having no bars')
$lines.Add('  and is not cropped. The crop actually used, and its source, are printed above')
$lines.Add('  for every pair. Retail software captures contain no HUD.')
$lines.Add('- Port side: `bevi-knight.exe --game <install> --start <level> --frames 90 --shot <png> --mute`')
$lines.Add('  via `tools/parity/port-capture.ps1`. Captures are rgb24 at the port window size: the')
$lines.Add('  first pass was 1280x720 (16:9); after the 2026-10 rebuild the port window is 1024x768')
$lines.Add('  (4:3). The exact size of each port image is read from the PNG (see above), and the')
$lines.Add('  port draws its HUD and notification text into the frame.')
$lines.Add('- Different renderers, palettes and FOVs. Both images are stretched onto one common')
$lines.Add(('  {0}x{1} grayscale canvas (`scale`, `setsar=1`, `format=gray`) before' -f $NormalizedWidth, $NormalizedHeight))
$lines.Add('  SSIM and mean absolute difference. This deliberately ignores each side aspect and FOV')
$lines.Add('  and the extra port HUD, so the number ranks pairs for triage; the side-by-side and')
$lines.Add('  contact sheet are the real evidence. The retail PNG also carries a 4:3 pixel')
$lines.Add('  sample-aspect tag (ffmpeg reports DAR 16:9); the normalization uses square pixels.')
$lines.Add('- Pose: shots.tsv `-` means the level spawn (plain `--start`, no `--shot-at`). A')
$lines.Add('  numeric row is passed to the port as `--shot-at "x,y,z,yaw"`; the port has horizontal')
$lines.Add('  aim only, so retail PYR pitch cannot be reproduced (known limitation).')
$lines.Add('- Port captures never run while JK.EXE is running; `port-capture.ps1` waits or aborts.')
$lines.Add('- Reproduce: `pwsh -File tools/parity/port-capture.ps1` then')
$lines.Add('  `pwsh -File tools/parity/compare.ps1`.')
$lines.Add('')

if ($missing.Count -gt 0) {
    $lines.Add('## Not captured yet')
    $lines.Add('')
    foreach ($m in $missing) {
        $what = @()
        if (-not $m.Retail) { $what += 'retail PNG missing' }
        if (-not $m.Port) { $what += 'port PNG missing' }
        $lines.Add("- $($m.Name): " + ($what -join '; '))
    }
    $lines.Add('')
}

if ($errors.Count -gt 0) {
    $lines.Add('## Failed')
    $lines.Add('')
    foreach ($e in $errors) {
        $lines.Add("- $($e.Name): " + $e.Message)
    }
    $lines.Add('')
}

Set-Content -LiteralPath $ReportPath -Value ($lines -join "`n") -Encoding utf8

Write-Host ''
Write-Host "compared: $($ordered.Count) pair(s); not captured yet: $($missing.Count); failed: $($errors.Count)"
Write-Host "report: $ReportPath"
if ($ordered.Count -gt 0) { Write-Host "contact sheet: $ContactPath" }
if ($errors.Count -gt 0) { exit 1 }
exit 0
