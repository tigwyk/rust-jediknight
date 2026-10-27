# retail-lib.ps1 — helpers for capturing the retail GOG Jedi Knight (JK.EXE).
# Used by tools/parity/retail-capture.ps1. Dependency-free beyond ffmpeg/ffprobe and PowerShell/.NET.
#
# Why SendInput scancodes: DirectInput titles ignore WM_KEYDOWN; injected scan codes reach them.
# Keyboard/menu facts and window classes are documented in target/parity/NOTES.md.

if (-not ('JkRetail' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class JkRetail {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)]
  public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit, Size = 40)]
  public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; }

  [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] p, int cb);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, StringBuilder l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll")] public static extern IntPtr GetFocus();
  [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool attach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);

  // WM_CLOSE on the game window runs the engine's own shutdown: it stops the level, shuts DirectDraw
  // down and restores the desktop display mode. Killing the process skips all of that (the monitor
  // stays in the 640x480 game mode), so the harness closes it this way first.
  public static void CloseWindow(IntPtr h) { PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); }

  const uint SCANCODE = 0x0008, KEYUP = 0x0002, EXTENDED = 0x0001;
  static void One(ushort scan, bool down, bool extended) {
    var i = new INPUT[1];
    i[0].type = 1; i[0].ki.wVk = 0; i[0].ki.wScan = scan;
    i[0].ki.dwFlags = SCANCODE | (down ? 0u : KEYUP) | (extended && down ? EXTENDED : 0u);
    SendInput(1, i, 40);
  }
  // Normal tap: 40 ms hold.
  public static void Key(ushort scan, bool extended) { One(scan, true, extended); System.Threading.Thread.Sleep(40); One(scan, false, extended); }
  // Slow hold for the in-game T console: key down, hold, key up (NOTES: >=300 ms hold, >=250 ms gap).
  public static void KeyHold(ushort scan, bool extended, int holdMs) {
    One(scan, true, extended); System.Threading.Thread.Sleep(holdMs); One(scan, false, extended);
  }
  // Scancode + virtual-key pair: needed to type into normal Win32 controls (the dev-console
  // Edit box) because only the virtual key produces WM_CHAR there.
  static void OneVk(ushort vk, ushort scan, bool down, bool extended) {
    var i = new INPUT[1];
    i[0].type = 1; i[0].ki.wVk = vk; i[0].ki.wScan = scan;
    i[0].ki.dwFlags = (down ? 0u : KEYUP) | (extended ? EXTENDED : 0u);
    SendInput(1, i, 40);
  }
  public static void KeyVk(ushort vk, ushort scan, bool extended) {
    OneVk(vk, scan, true, extended); System.Threading.Thread.Sleep(40); OneVk(vk, scan, false, extended);
  }
  public static void Click(int x, int y) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(150);
    mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(80);
    mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(150);
  }
  public static string Text(IntPtr h) { var sb = new StringBuilder(1024); GetWindowText(h, sb, 1024); return sb.ToString(); }
  public static string ClassName(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  public static IntPtr FindWindowForPid(uint pid, string title) {
    IntPtr res = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
      var sb = new StringBuilder(512); GetWindowText(h, sb, 512);
      if (title == null || sb.ToString() == title) { res = h; return false; }
      return true;
    }, IntPtr.Zero);
    return res;
  }
  public static List<IntPtr> WindowsForPid(uint pid) {
    var r = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) r.Add(h); return true; }, IntPtr.Zero);
    return r;
  }
  public static List<IntPtr> Children(IntPtr parent) {
    var r = new List<IntPtr>();
    // Child enumeration via EnumChildWindows
    EnumChildProc cb = (h, l) => { r.Add(h); return true; };
    EnumChildWindows(parent, cb, IntPtr.Zero);
    return r;
  }
  public delegate bool EnumChildProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumChildProc cb, IntPtr l);
  public static int ListBoxCount(IntPtr lb) { return (int)SendMessage(lb, 0x018B, IntPtr.Zero, IntPtr.Zero); }
  public static string ListBoxText(IntPtr lb, int index) {
    var sb = new StringBuilder(8192); SendMessage(lb, 0x0189, (IntPtr)index, sb); return sb.ToString();
  }
  public static string WindowRectText(IntPtr h) {
    RECT r; GetWindowRect(h, out r); return String.Format("({0},{1})-({2},{3})", r.Left, r.Top, r.Right, r.Bottom);
  }
  public static string GetWindowTextCross(IntPtr h) {
    var sb = new StringBuilder(4096); SendMessage(h, 0x000D, (IntPtr)4096, sb); return sb.ToString(); // WM_GETTEXT
  }
  // The dev-console dialog only runs a command when its Edit control (id 1024) has focus in the
  // game's own input queue, so cross-process focus is forced for the duration of SetFocus.
  public static void ForceFocus(IntPtr target) {
    uint myThread = GetCurrentThreadId();
    uint pid;
    uint targetThread = GetWindowThreadProcessId(target, out pid);
    if (targetThread == 0) return;
    AttachThreadInput(myThread, targetThread, true);
    try { SetFocus(target); } finally { AttachThreadInput(myThread, targetThread, false); }
  }
  public static void SetControlText(IntPtr h, string text) { SendMessage(h, 0x000C, IntPtr.Zero, text); } // WM_SETTEXT
  public static string ForegroundTitle() {
    var h = GetForegroundWindow(); var sb = new StringBuilder(512); GetWindowText(h, sb, 512);
    return h.ToString() + ":'" + sb.ToString() + "'";
  }
}
'@
}

$script:JkScan = @{
  ESC = 0x01; ENTER = 0x1C; SPACE = 0x39; TAB = 0x0F; BACKSPACE = 0x0E
  '1' = 0x02; '2' = 0x03; '3' = 0x04; '4' = 0x05; '5' = 0x06
  '6' = 0x07; '7' = 0x08; '8' = 0x09; '9' = 0x0A; '0' = 0x0B
  MINUS = 0x0C; PERIOD = 0x34; GRAVE = 0x29; T = 0x14
  F1 = 0x3B; F2 = 0x3C; F3 = 0x3D; F4 = 0x3E; F5 = 0x3F; F6 = 0x40
  F7 = 0x41; F8 = 0x42; F9 = 0x43; F10 = 0x44; F11 = 0x57; F12 = 0x58
  UP = 0xC8; DOWN = 0xD0; LEFT = 0xCB; RIGHT = 0xCD
}
$script:JkExtended = @('UP', 'DOWN', 'LEFT', 'RIGHT')

function Write-JkLog {
  param([Parameter(Mandatory)][string]$Message, [string]$LogPath)
  $line = '{0:HH:mm:ss.fff} {1}' -f (Get-Date), $Message
  Write-Host $line
  if ($LogPath) { Add-Content -LiteralPath $LogPath -Value $line -Encoding utf8 }
}

function Send-JkKey {
  param([Parameter(Mandatory)][string]$Name)
  $key = $Name.ToUpperInvariant()
  if (-not $script:JkScan.ContainsKey($key)) { throw "no scancode for key '$Name'" }
  $extended = $script:JkExtended -contains $key
  [JkRetail]::Key([ushort]$script:JkScan[$key], $extended)
}

# Slow, held typing: the in-game T console drops keys sent faster than this (NOTES.md).
function Send-JkTextSlow {
  param([Parameter(Mandatory)][string]$Text, [int]$HoldMs = 320, [int]$GapMs = 270)
  foreach ($ch in $Text.ToCharArray()) {
    $name = switch -Regex ($ch.ToString()) {
      '[a-zA-Z]' { $ch.ToString().ToUpperInvariant() }
      '[0-9]'    { $ch.ToString() }
      ' '        { 'SPACE' }
      '-'        { 'MINUS' }
      '\.'       { 'PERIOD' }
      default    { throw "no scancode for character '$ch'" }
    }
    $extended = $script:JkExtended -contains $name
    [JkRetail]::KeyHold([ushort]$script:JkScan[$name], $extended, $HoldMs)
    Start-Sleep -Milliseconds $GapMs
  }
}

function Send-JkClick {
  param([Parameter(Mandatory)][int]$X, [Parameter(Mandatory)][int]$Y)
  [JkRetail]::Click($X, $Y)
}

function Get-JkWindow {
  param([Parameter(Mandatory)][int]$ProcessId, [string]$Title = 'Jedi Knight')
  [JkRetail]::FindWindowForPid([uint32]$ProcessId, $Title)
}

# Console dialog window for a running JK, or IntPtr.Zero.
function Get-JkConsoleWindow {
  param([Parameter(Mandatory)][int]$ProcessId)
  [JkRetail]::FindWindowForPid([uint32]$ProcessId, 'Jedi Console')
}

function Get-JkConsoleListBox {
  param([Parameter(Mandatory)][int]$ProcessId)
  $console = Get-JkConsoleWindow -ProcessId $ProcessId
  if ($console -eq [IntPtr]::Zero) { return [IntPtr]::Zero }
  $lb = [JkRetail]::GetDlgItem($console, 1037)
  if ($lb -ne [IntPtr]::Zero -and [JkRetail]::IsWindow($lb)) { return $lb }
  return [IntPtr]::Zero
}

# All lines currently in the dev-console ListBox (id 1037), as strings.
function Get-JkConsoleLines {
  param([Parameter(Mandatory)][int]$ProcessId)
  $lb = Get-JkConsoleListBox -ProcessId $ProcessId
  if ($lb -eq [IntPtr]::Zero) { return @() }
  $count = [JkRetail]::ListBoxCount($lb)
  if ($count -le 0) { return @() }
  $lines = New-Object System.Collections.Generic.List[string]
  for ($i = 0; $i -lt $count; $i++) { $lines.Add([JkRetail]::ListBoxText($lb, $i)) }
  return $lines.ToArray()
}

# Wait until a ListBox line matches $Pattern. Returns the matching line (or $null on timeout).
function Wait-JkConsoleLine {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Pattern,
    [int]$TimeoutSec = 45,
    [int]$AfterIndex = 0
  )
  $stopwatch = [Diagnostics.Stopwatch]::StartNew()
  while ($stopwatch.Elapsed.TotalSeconds -lt $TimeoutSec) {
    $lines = @(Get-JkConsoleLines -ProcessId $ProcessId)
    for ($i = $AfterIndex; $i -lt $lines.Count; $i++) {
      if ($lines[$i] -match $Pattern) { return $lines[$i] }
    }
    Start-Sleep -Milliseconds 400
  }
  return $null
}

# ffmpeg/ffprobe helpers -----------------------------------------------------

function Invoke-JkFfmpeg {
  param([Parameter(Mandatory)][string[]]$Arguments, [Parameter(Mandatory)][string]$LogPath)
  & ffmpeg -hide_banner -nostdin @Arguments 2>&1 | ForEach-Object {
    $text = "$_"
    Add-Content -LiteralPath $LogPath -Value ("  ffmpeg| " + $text) -Encoding utf8
  }
  return $LASTEXITCODE
}

function Convert-JkShotToPng {
  param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$Destination, [Parameter(Mandatory)][string]$LogPath)
  if (Test-Path -LiteralPath $Destination) { Remove-Item -LiteralPath $Destination -Force }
  $code = Invoke-JkFfmpeg -Arguments @('-loglevel', 'error', '-y', '-i', $Source, $Destination) -LogPath $LogPath
  if ($code -ne 0 -or -not (Test-Path -LiteralPath $Destination)) {
    throw "ffmpeg failed to convert '$Source' to '$Destination' (exit $code)"
  }
}

# Returns @{ Width; Height; PixFmt; Bytes; Sha256; Yavg; Crop = "W:H:X:Y" } for a PNG.
# Yavg is luma average (signalstats); Crop is the non-black content box (bbox min_val=16).
function Get-JkPngStats {
  param([Parameter(Mandatory)][string]$Path, [string]$LogPath = $null)
  if (-not (Test-Path -LiteralPath $Path)) { throw "PNG not found: $Path" }
  $probe = & ffprobe -v error -select_streams v:0 -show_entries stream=width,height,pix_fmt -of csv=p=0 $Path 2>$null
  $parts = "$probe".Trim().Split(',')
  if ($parts.Count -lt 3) { throw "ffprobe could not read '$Path'" }
  $file = Get-Item -LiteralPath $Path
  $sha = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  $err = & ffmpeg -hide_banner -nostats -i $Path -vf 'signalstats,metadata=print:key=lavfi.signalstats.YAVG,bbox=min_val=16' -frames:v 1 -f null - 2>&1
  $yavg = $null; $crop = $null
  foreach ($line in $err) {
    $text = "$line"
    if ($text -match 'lavfi\.signalstats\.YAVG=([0-9.]+)') { $yavg = [double]$Matches[1] }
    if ($text -match 'crop=(\d+):(\d+):(\d+):(\d+)') { $crop = $Matches[0] -replace '^crop=', '' }
  }
  return [pscustomobject]@{
    Path = $file.FullName; Width = [int]$parts[0]; Height = [int]$parts[1]; PixFmt = $parts[2]
    Bytes = $file.Length; Sha256 = $sha; Yavg = $yavg; Crop = $crop
  }
}

# Screenshot the live game window (used only for diagnostics/state checks).
function Save-JkWindowSnapshot {
  param([Parameter(Mandatory)][string]$Path, [string]$LogPath = $null)
  if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }
  $null = Invoke-JkFfmpeg -Arguments @('-loglevel', 'error', '-y', '-f', 'gdigrab', '-draw_mouse', '0', '-i', 'title=Jedi Knight', '-frames:v', '1', $Path) -LogPath $LogPath
  return (Test-Path -LiteralPath $Path)
}

# Average luma of the current game window, or $null if the window cannot be grabbed.
function Get-JkWindowYavg {
  param([Parameter(Mandatory)][string]$ScratchDir, [string]$LogPath = $null)
  $tmp = Join-Path $ScratchDir ('state-{0}.png' -f [guid]::NewGuid().ToString('N').Substring(0, 8))
  try {
    if (-not (Save-JkWindowSnapshot -Path $tmp -LogPath $LogPath)) { return $null }
    $stats = Get-JkPngStats -Path $tmp -LogPath $LogPath
    return $stats.Yavg
  } finally {
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }
  }
}

# Registry helpers -----------------------------------------------------------

$script:VirtualStoreKey = 'HKCU:\Software\Classes\VirtualStore\MACHINE\SOFTWARE\WOW6432Node\LucasArts Entertainment Company\JediKnight\v1.0'
$script:HklmJkKey = 'HKLM:\SOFTWARE\WOW6432Node\LucasArts Entertainment Company\JediKnight\v1.0'

function ConvertFrom-JkRegDword {
  param($Value)
  if ($null -eq $Value) { return $null }
  if ($Value -is [byte[]]) {
    if ($Value.Length -lt 4) { return $null }
    return [int][BitConverter]::ToUInt32($Value, 0)
  }
  return [int]$Value
}

# Executable patch -----------------------------------------------------------
#
# The dev-console verbs beyond `coords` -- `warp`, `cogstatus`, `aistatus`, `slowmo`, ... -- register
# only when bit 0x100 of the debug-flags word at 0x8EE66C is set. Nothing sets it at startup: the only
# writers are the `setdebugmodeflags`/`cleardebugmodeflags` cog verbs, and registration happens once in
# `sithCommand_Startup` (called from `sithConsole_Startup`), before any level script can run. So
# `warp` is "not recognized" in the retail capture unless the executable is patched.
#
# At 0x4EDC99 the engine tests that bit and jumps over the registration block: `0f 84 b8 01 00 00` at
# file offset 0xED0A4. Replacing those six bytes with NOPs makes the branch always fall through and
# registers everything. The six jump bytes alone occur three times in JK.EXE; the `test al,1` in
# front (`f6 c4 01`) makes the 9-byte site unique, and that is what gets verified before writing.
# Only the staged scratch copy is patched -- the real install is never modified.
function Set-JkDevCommandsEnabled {
  param([Parameter(Mandatory)][string]$ExePath, [string]$LogPath = $null)
  $siteOffset = 0xED0A4
  # The signature starts three bytes before the jump, at the `test al,1`.
  $signatureOffset = $siteOffset - 3
  # `test al,1` then `jz +0x1b8`: unique with the flag test in front (the jz bytes alone are not).
  $signature = [byte[]](0xf6, 0xc4, 0x01, 0x0f, 0x84, 0xb8, 0x01, 0x00, 0x00)
  $nop = [byte]0x90
  $bytes = [System.IO.File]::ReadAllBytes($ExePath)
  if ($bytes.Length -le ($siteOffset + 6)) {
    throw ('dev-command patch: ' + $ExePath + ' is too small to be JK.EXE')
  }
  # Already patched? `test al,1` followed by NOPs. Checked before the signature search, because the
  # signature requires the original jump bytes.
  $already = $true
  for ($i = 0; $i -lt 3; $i++) {
    if ($bytes[$siteOffset - 3 + $i] -ne $signature[$i]) { $already = $false; break }
  }
  if ($already) {
    for ($i = 0; $i -lt 6; $i++) {
      if ($bytes[$siteOffset + $i] -ne $nop) { $already = $false; break }
    }
  }
  if ($already) {
    Write-JkLog -Message 'dev-console commands already unlocked in the staged JK.EXE' -LogPath $LogPath
    return $false
  }
  $hits = New-Object System.Collections.Generic.List[int]
  for ($i = 0; $i -le $bytes.Length - $signature.Length; $i++) {
    $same = $true
    for ($j = 0; $j -lt $signature.Length; $j++) {
      if ($bytes[$i + $j] -ne $signature[$j]) { $same = $false; break }
    }
    if ($same) { $hits.Add($i) }
  }
  if ($hits.Count -ne 1) {
    throw ('dev-command patch: expected the test-al/jz signature exactly once in ' + $ExePath + ', found ' + $hits.Count)
  }
  if ($hits[0] -ne $signatureOffset) {
    throw ('dev-command patch: signature is at file offset 0x{0:X}, expected 0x{1:X} (unexpected JK.EXE build?)' -f $hits[0], $signatureOffset)
  }
  for ($i = 0; $i -lt 6; $i++) { $bytes[$siteOffset + $i] = $nop }
  [System.IO.File]::WriteAllBytes($ExePath, $bytes)
  $check = [System.IO.File]::ReadAllBytes($ExePath)
  if ($check.Length -ne $bytes.Length) { throw 'dev-command patch: JK.EXE size changed after writing' }
  for ($i = 0; $i -lt 6; $i++) {
    if ($check[$siteOffset + $i] -ne $nop) { throw 'dev-command patch: write-back verification failed' }
  }
  Write-JkLog -Message 'dev-console commands unlocked: NOPed the debug-flag jump at JK.EXE file offset 0xED0A4 (warp, cogstatus, aistatus, slowmo, ...)' -LogPath $LogPath
  return $true
}

# NOTES.md: the effective 3D-acceleration switch for this virtualized legacy app.
# Ensure HKCU VirtualStore b3DAccel = 0 (never write HKLM).
function Set-JkVirtualStore3DOff {
  param([string]$LogPath = $null)
  if (-not (Test-Path $script:VirtualStoreKey)) {
    New-Item -Path $script:VirtualStoreKey -Force | Out-Null
  }
  $existing = $null
  try { $existing = (Get-ItemProperty -Path $script:VirtualStoreKey -Name 'b3DAccel' -ErrorAction Stop).b3DAccel } catch { }
  $effective = ConvertFrom-JkRegDword $existing
  if ($null -eq $effective -or $effective -ne 0) {
    New-ItemProperty -Path $script:VirtualStoreKey -Name 'b3DAccel' -PropertyType Binary -Value ([byte[]](0, 0, 0, 0)) -Force | Out-Null
    Write-JkLog -Message 'VirtualStore b3DAccel set to 0 (3D acceleration off)' -LogPath $LogPath
  } else {
    Write-JkLog -Message 'VirtualStore b3DAccel already 0 (3D acceleration off)' -LogPath $LogPath
  }
}

# Effective viewSize (5 = engine default; the number comes from VirtualStore if set, else HKLM).
function Get-JkEffectiveViewSize {
  $value = $null
  try { $value = (Get-ItemProperty -Path $script:VirtualStoreKey -Name 'viewSize' -ErrorAction Stop).viewSize } catch { }
  if ($null -eq $value) {
    try { $value = (Get-ItemProperty -Path $script:HklmJkKey -Name 'viewSize' -ErrorAction Stop).viewSize } catch { }
  }
  return (ConvertFrom-JkRegDword $value)
}

# Effective displayMode index and b3DAccel flag (VirtualStore first, then HKLM).
function Get-JkEffectiveDisplayMode {
  $value = $null
  try { $value = (Get-ItemProperty -Path $script:VirtualStoreKey -Name 'displayMode' -ErrorAction Stop).displayMode } catch { }
  return (ConvertFrom-JkRegDword $value)
}
function Get-JkEffective3DAccel {
  $value = $null
  try { $value = (Get-ItemProperty -Path $script:VirtualStoreKey -Name 'b3DAccel' -ErrorAction Stop).b3DAccel } catch { }
  return (ConvertFrom-JkRegDword $value)
}

# Ensure HKCU VirtualStore b3DAccel = 1 (never write HKLM). Used with the dgVoodoo wrapper: the
# engine then renders at the display mode's 16-bit path and F12 writes a 24-bit JSHOT##.BMP.
function Set-JkVirtualStore3DOn {
  param([string]$LogPath = $null)
  if (-not (Test-Path $script:VirtualStoreKey)) {
    New-Item -Path $script:VirtualStoreKey -Force | Out-Null
  }
  $existing = $null
  try { $existing = (Get-ItemProperty -Path $script:VirtualStoreKey -Name 'b3DAccel' -ErrorAction Stop).b3DAccel } catch { }
  $effective = ConvertFrom-JkRegDword $existing
  if ($null -eq $effective -or $effective -ne 1) {
    New-ItemProperty -Path $script:VirtualStoreKey -Name 'b3DAccel' -PropertyType Binary -Value ([byte[]](1, 0, 0, 0)) -Force | Out-Null
    Write-JkLog -Message 'VirtualStore b3DAccel set to 1 (3D acceleration on)' -LogPath $LogPath
  } else {
    Write-JkLog -Message 'VirtualStore b3DAccel already 1 (3D acceleration on)' -LogPath $LogPath
  }
}

# The retail build's Direct3D path calls the system `D3DIM.DLL`, which crashes on Windows 11
# (APPCRASH: JK.EXE faulting module D3DIM.DLL, 0xc0000005 — reproduce: run a level with
# b3DAccel=1 and no wrapper). dgVoodoo2 replaces `DDraw.dll`/`D3DImm.dll` with its own Direct3D
# 11 translation, which renders the game's 16-bit path; F12 then writes a 24-bit BMP instead of
# the 8-bit palettised PCX, so the reference frames have no dither. This stages the wrapper into
# the scratch copy only and caches the download under target/parity/wrapper.
$script:DgVoodooVersion = '2.87.6'
$script:DgVoodooUrl = 'https://github.com/dege-diosg/dgVoodoo2/releases/download/2.87.6/dgVoodoo2_87_6.zip'
$script:DgVoodooSha256 = '9AFA493396FEEC7108F42A2D97661EEF96BBAAC5ADDF0DFB5CFBC3001CD76A0A'
function Install-Jk3DWrapper {
  param([Parameter(Mandatory)][string]$Destination, [string]$LogPath = $null)
  $repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
  $cache = Join-Path $repo 'target/parity/wrapper'
  $x86 = Join-Path $cache 'x/MS/x86'
  $zip = Join-Path $cache ('dgVoodoo2_{0}.zip' -f ($script:DgVoodooVersion -replace '\.', '_'))
  if (-not (Test-Path (Join-Path $x86 'DDraw.dll'))) {
    New-Item -ItemType Directory -Force -Path $cache | Out-Null
    if (-not (Test-Path $zip)) {
      Write-JkLog -Message "3D wrapper: downloading dgVoodoo2 $($script:DgVoodooVersion)" -LogPath $LogPath
      try {
        Invoke-WebRequest -Uri $script:DgVoodooUrl -OutFile $zip -UseBasicParsing
      } catch {
        throw ('3D wrapper download failed: ' + $_.Exception.Message + ' — drop the dgVoodoo2 zip at ' + $zip + ' and rerun, or run with -Software for the 8-bit path')
      }
    }
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
    if ($hash -ne $script:DgVoodooSha256) {
      throw ('3D wrapper zip does not match dgVoodoo2 ' + $script:DgVoodooVersion + ' (sha256=' + $hash + ')')
    }
    $extract = Join-Path $cache 'x'
    if (Test-Path $extract) { Remove-Item -Recurse -Force $extract }
    Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
    if (-not (Test-Path (Join-Path $x86 'DDraw.dll'))) { throw '3D wrapper zip did not contain MS/x86/DDraw.dll' }
    Write-JkLog -Message '3D wrapper: dgVoodoo2 cached under target/parity/wrapper' -LogPath $LogPath
  }
  # Tuned config: point filtering (no smoothing, so texture detail matches the software renderer)
  # and no watermark. Also re-enable Alt+Enter so the windowed/display-mode behaviour is normal.
  $confCache = Join-Path $cache 'dgVoodoo.conf'
  if (-not (Test-Path $confCache)) {
    $conf = Get-Content -LiteralPath (Join-Path $cache 'x/dgVoodoo.conf') -Raw
    $conf = $conf -replace '(?m)^Filtering\s*=\s*appdriven', 'Filtering                           = pointsampled'
    $conf = $conf -replace '(?m)^dgVoodooWatermark\s*=\s*true', 'dgVoodooWatermark                   = false'
    Set-Content -LiteralPath $confCache -Value $conf -Encoding ascii
  }
  Copy-Item -LiteralPath (Join-Path $x86 'D3DImm.dll') -Destination (Join-Path $Destination 'D3DImm.dll') -Force
  Copy-Item -LiteralPath (Join-Path $x86 'DDraw.dll') -Destination (Join-Path $Destination 'DDraw.dll') -Force
  Copy-Item -LiteralPath $confCache -Destination (Join-Path $Destination 'dgVoodoo.conf') -Force
  foreach ($name in 'D3DImm.dll', 'DDraw.dll') {
    $path = Join-Path $Destination $name
    if (-not (Test-Path $path) -or (Get-Item $path).Length -lt 100000) {
      throw "3D wrapper stage failed: $path is missing or too small"
    }
  }
  Write-JkLog -Message '3D wrapper staged in the scratch copy (D3DImm.dll, DDraw.dll, dgVoodoo.conf; point filtering, no watermark)' -LogPath $LogPath
}

