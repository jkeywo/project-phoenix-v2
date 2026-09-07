[CmdletBinding()]
param([Parameter(Mandatory)][string]$Output)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Output) { throw 'Hardware record already exists.' }
# The DPI context applies only to this helper process. It prevents virtualized
# desktop coordinates from being mistaken for physical render dimensions.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class PhoenixProfileDisplays {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left, top, right, bottom; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct Info {
        public int size; public Rect monitor, work; public uint flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string device;
    }
    public class Display {
        public string device { get; set; }
        public string identity { get; set; }
        public int width { get; set; }
        public int height { get; set; }
        public int x { get; set; }
        public int y { get; set; }
        public int scalePercent { get; set; }
        public double dpi { get; set; }
        public bool primary { get; set; }
    }
    private delegate bool Callback(IntPtr monitor, IntPtr dc, IntPtr rect, IntPtr data);
    [DllImport("user32.dll")] private static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] private static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr clip, Callback callback, IntPtr data);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern bool GetMonitorInfo(IntPtr monitor, ref Info info);
    [DllImport("shcore.dll")] private static extern int GetScaleFactorForMonitor(IntPtr monitor, out int scale);
    public static Display[] Read() {
        SetProcessDpiAwarenessContext(new IntPtr(-4));
        var displays = new List<Display>();
        Callback callback = (monitor, dc, rect, data) => {
            var info = new Info { size = Marshal.SizeOf<Info>() };
            if (!GetMonitorInfo(monitor, ref info)) throw new InvalidOperationException("GetMonitorInfo failed");
            int scale;
            if (GetScaleFactorForMonitor(monitor, out scale) != 0) throw new InvalidOperationException("Monitor scale unavailable");
            int width = info.monitor.right-info.monitor.left, height = info.monitor.bottom-info.monitor.top;
            displays.Add(new Display { device=info.device, identity=info.device+"@"+width+"x"+height,
                width=width, height=height, x=info.monitor.left, y=info.monitor.top,
                scalePercent=scale, dpi=96.0*scale/100, primary=(info.flags&1)!=0 });
            return true;
        };
        if (!EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, callback, IntPtr.Zero)) throw new InvalidOperationException("Display enumeration failed");
        return displays.ToArray();
    }
}
'@
$gpus = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,DriverDate,VideoModeDescription)
$record = [ordered]@{
    capturedUtc=[DateTime]::UtcNow.ToString('o')
    gpu=($gpus.Name -join ' / '); driver=($gpus.DriverVersion -join ' / ')
    adapters=$gpus; powerMode=((& powercfg /getactivescheme) -join ' ')
    displays=@([PhoenixProfileDisplays]::Read())
    note='Physical desktop modes and effective OS scale; actual wgpu adapter selection must also be retained from the host log.'
}
$record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding UTF8
$record | ConvertTo-Json -Depth 8
