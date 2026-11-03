# Display-mode helpers shared by the parity harness.
#
# The retail game is a fullscreen DirectDraw application: it switches the primary monitor to a
# legacy mode (640x480 here) and restores the desktop when it shuts down *normally*. The harness
# therefore closes the game with WM_CLOSE first (see retail-capture.ps1) instead of killing it.
# This file is the belt-and-braces for the cases where that cannot happen (crash, hang, Ctrl+C):
# it snapshots the desktop modes before a run and can put them back afterwards.
#
# Dot-source it:   . "$PSScriptRoot\display.ps1"
# Functions:       Get-JkDisplayState, Save-JkDisplayState, Restore-JkDisplayState,
#                  Test-JkDisplayMatches, Confirm-JkDesktop, Restore-JkRegistryDefaults

$script:JkDisplaySource = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class JkDisplay
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct DEVMODE
    {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
        public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
        public int dmFields;
        public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
        public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
        public short dmLogPixels;
        public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
        public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2,
                   dmPanningWidth, dmPanningHeight;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct DISPLAY_DEVICE
    {
        public int cb;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceString;
        public int StateFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceID;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceKey;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern bool EnumDisplayDevices(string lpDevice, uint iDevNum, ref DISPLAY_DEVICE lpDisplayDevice, uint dwFlags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern bool EnumDisplaySettingsEx(string lpszDeviceName, int iModeNum, ref DEVMODE lpDevMode, uint dwFlags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int ChangeDisplaySettingsEx(string lpszDeviceName, ref DEVMODE lpDevMode, IntPtr hwnd, uint dwflags, IntPtr lParam);

    const int ENUM_CURRENT_SETTINGS  = -1;
    const int ENUM_REGISTRY_SETTINGS = -2;
    const int DM_POSITION = 0x00000020, DM_BITSPERPEL = 0x00040000, DM_PELSWIDTH = 0x00080000,
              DM_PELSHEIGHT = 0x00100000, DM_DISPLAYFREQUENCY = 0x00400000;
    const int DISP_CHANGE_SUCCESSFUL = 0;

    public enum Kind { Current, Registry }

    public static DEVMODE Read(string device, Kind kind)
    {
        var dm = new DEVMODE();
        dm.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
        EnumDisplaySettingsEx(device, kind == Kind.Current ? ENUM_CURRENT_SETTINGS : ENUM_REGISTRY_SETTINGS, ref dm, 0);
        return dm;
    }

    // One entry per active output, in device order.
    public static List<string> Devices()
    {
        var names = new List<string>();
        for (uint i = 0; ; i++)
        {
            var dd = new DISPLAY_DEVICE();
            dd.cb = Marshal.SizeOf(typeof(DISPLAY_DEVICE));
            if (!EnumDisplayDevices(null, i, ref dd, 0)) break;
            if ((dd.StateFlags & 0x1) == 0) continue;      // DISPLAY_DEVICE_ATTACHED_TO_DESKTOP
            names.Add(dd.DeviceName);
        }
        return names;
    }

    public static string SnapshotJson(Kind kind)
    {
        var sb = new StringBuilder();
        sb.Append("[");
        bool first = true;
        foreach (var device in Devices())
        {
            var dm = Read(device, kind);
            if (!first) sb.Append(",");
            first = false;
            sb.AppendFormat("{{\"device\":\"{0}\",\"width\":{1},\"height\":{2},\"frequency\":{3},\"bpp\":{4},\"x\":{5},\"y\":{6}}}",
                device.Replace("\\", "\\\\"), dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency,
                dm.dmBitsPerPel, dm.dmPositionX, dm.dmPositionY);
        }
        sb.Append("]");
        return sb.ToString();
    }

    public static string Summary()
    {
        var sb = new StringBuilder();
        foreach (var device in Devices())
        {
            var dm = Read(device, Kind.Current);
            var reg = Read(device, Kind.Registry);
            if (sb.Length > 0) sb.Append("; ");
            sb.AppendFormat("{0} {1}x{2}@{3} {4}bpp (saved default {5}x{6}@{7} {8}bpp)",
                device, dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency, dm.dmBitsPerPel,
                reg.dmPelsWidth, reg.dmPelsHeight, reg.dmDisplayFrequency, reg.dmBitsPerPel);
        }
        return sb.ToString();
    }

    // Apply one mode per device; returns "device=code" for each (0 == success).
    public static string Apply(string device, int width, int height, int frequency, int bpp)
    {
        var dm = Read(device, Kind.Current);
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_BITSPERPEL;
        dm.dmPelsWidth = width;
        dm.dmPelsHeight = height;
        dm.dmDisplayFrequency = frequency;
        dm.dmBitsPerPel = bpp;
        int rc = ChangeDisplaySettingsEx(device, ref dm, IntPtr.Zero, 0, IntPtr.Zero);  // session only, not persisted
        return device + "=" + rc + (rc == DISP_CHANGE_SUCCESSFUL ? " (ok)" : " (failed)");
    }

    // Put every output back to the mode Windows has stored as its default (the mode it boots with).
    // Reuses the same size/frequency/bpp apply path as a saved-state restore; output positions are
    // left alone (the game only changes the mode, never the layout).
    public static string ApplyRegistryDefaults()
    {
        var results = new List<string>();
        foreach (var device in Devices())
        {
            var reg = Read(device, Kind.Registry);
            if (reg.dmPelsWidth <= 0 || reg.dmPelsHeight <= 0) { results.Add(device + "=no-default"); continue; }
            results.Add(Apply(device, reg.dmPelsWidth, reg.dmPelsHeight, reg.dmDisplayFrequency, reg.dmBitsPerPel));
        }
        return string.Join("; ", results);
    }
}
'@

if (-not ('JkDisplay' -as [type])) {
    Add-Type -TypeDefinition $script:JkDisplaySource -ErrorAction Stop
}

function Get-JkDisplayState {
    <# Current desktop modes, one object per active display. #>
    return @([JkDisplay]::SnapshotJson([JkDisplay+Kind]::Current) | ConvertFrom-Json)
}

function Save-JkDisplayState {
    <# Write the current desktop modes to a JSON file used to recover a run that dies badly. #>
    param([Parameter(Mandatory)][string]$Path)
    [ordered]@{
        saved_at = (Get-Date -Format o)
        modes    = @(Get-JkDisplayState)
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Path -Encoding utf8
    return $Path
}

function Test-JkDisplayMatches {
    <# True when the current modes equal the saved ones (same size/frequency/bpp per device). #>
    param([Parameter(Mandatory)]$Saved)
    $now = Get-JkDisplayState
    if (@($Saved).Count -ne @($now).Count) { return $false }
    for ($i = 0; $i -lt @($Saved).Count; $i++) {
        $a = @($Saved)[$i]; $b = @($now)[$i]
        if ($a.device -ne $b.device -or $a.width -ne $b.width -or $a.height -ne $b.height -or
            $a.frequency -ne $b.frequency -or $a.bpp -ne $b.bpp) { return $false }
    }
    return $true
}

function Restore-JkDisplayState {
    <# Put the saved desktop modes back. Returns the per-device result lines and the post-restore state. #>
    param([Parameter(Mandatory)]$Saved)
    $results = foreach ($mode in @($Saved)) {
        [JkDisplay]::Apply([string]$mode.device, [int]$mode.width, [int]$mode.height,
                           [int]$mode.frequency, [int]$mode.bpp)
    }
    Start-Sleep -Milliseconds 600
    return [pscustomobject]@{ applied = @($results); now = (Get-JkDisplayState) }
}

function Restore-JkRegistryDefaults {
    <# Last resort: put every output back to the mode Windows has stored for it. #>
    $result = [JkDisplay]::ApplyRegistryDefaults()
    Start-Sleep -Milliseconds 600
    return [pscustomobject]@{ applied = $result; now = (Get-JkDisplayState) }
}

# Modes the 1997 engine can switch the primary output to. A desktop is very unlikely to be using
# one of these on purpose, so seeing one with no saved state means a run left the monitor there.
$script:JkLegacyModes = @('640x480', '640x400', '512x384', '320x240', '320x200', '320x400',
                          '720x480', '720x576', '800x600', '1024x768', '1152x864', '1280x960', '1280x1024',
                          '1280x720')

function Confirm-JkDesktop {
    <#
      Make sure the desktop is usable before launching or capturing an engine.
      - saved state exists -> restore it if the current modes differ;
      - no saved state      -> restore Windows' saved defaults only when the primary sits in a legacy
                               fullscreen-game mode (never touches an ordinary desktop resolution).
      Returns a small report; callers log it. Refuses to touch the display while JK.EXE is running.
    #>
    param([string]$StatePath)
    if (Get-Process -Name 'JK' -ErrorAction SilentlyContinue) {
        return [pscustomobject]@{ changed = $false; reason = 'JK.EXE is running; display left alone'; state = (Get-JkDisplayState) }
    }
    $primary = @(Get-JkDisplayState)[0]
    $size = '{0}x{1}' -f $primary.width, $primary.height
    # Only ever touch a desktop that is sitting in one of the engine's legacy fullscreen modes
    # (that is the signature of a killed run). An ordinary desktop resolution is left alone, so a
    # deliberate monitor change between runs is never clobbered.
    $isLegacy = ($script:JkLegacyModes -contains $size) -and $primary.device -eq '\\.\DISPLAY1'
    if (-not $isLegacy) {
        return [pscustomobject]@{ changed = $false; reason = "desktop $size is fine"; state = (Get-JkDisplayState) }
    }
    if ($StatePath -and (Test-Path -LiteralPath $StatePath)) {
        $saved = (Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json).modes
        $restore = Restore-JkDisplayState -Saved $saved
        return [pscustomobject]@{ changed = $true; reason = "primary was left in legacy game mode $size; restored the saved desktop mode ($($restore.applied -join ', '))"; state = $restore.now }
    }
    $restore = Restore-JkRegistryDefaults
    return [pscustomobject]@{ changed = $true; reason = "primary was left in legacy game mode $size; restored the Windows saved default ($($restore.applied))"; state = $restore.now }
}
