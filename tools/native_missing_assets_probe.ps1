param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference='Stop'
Add-Type @'
using System;using System.Text;using System.Collections.Generic;using System.Runtime.InteropServices;
public static class MissingAssetProbe {
 public delegate bool EnumProc(IntPtr window,IntPtr data);
 [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr window,EnumProc proc,IntPtr data);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window,StringBuilder text,int count);
 [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls,string title);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
 [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window,int id);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window,uint msg,IntPtr wp,IntPtr lp);
 public static string Children(IntPtr parent){var result=new List<string>();EnumChildWindows(parent,(h,p)=>{var t=new StringBuilder(1024);GetWindowText(h,t,t.Capacity);result.Add(h.ToInt64()+" id="+GetDlgCtrlID(h)+" "+t);return true;},IntPtr.Zero);return string.Join("\n",result);}
}
'@
$exe=(Resolve-Path -LiteralPath $Executable).Path
$start=[Diagnostics.ProcessStartInfo]::new($exe)
$start.UseShellExecute=$false;$start.CreateNoWindow=$true
$process=[Diagnostics.Process]::Start($start)
try {
    $dialog=[IntPtr]::Zero
    for($i=0;$i -lt 40;$i++) {
        Start-Sleep -Milliseconds 250
        $found=[MissingAssetProbe]::FindWindow('#32770','Project Nera - startup error')
        [uint32]$id=0;[void][MissingAssetProbe]::GetWindowThreadProcessId($found,[ref]$id)
        if($id -eq $process.Id){$dialog=$found;break}
    }
    if($dialog -eq [IntPtr]::Zero){throw 'Missing-assets error dialog was not shown.'}
    $button=[MissingAssetProbe]::GetDlgItem($dialog,1)
    if($button -eq [IntPtr]::Zero){$button=[MissingAssetProbe]::GetDlgItem($dialog,2)}
    for($i=0;$button -eq [IntPtr]::Zero -and $i -lt 40;$i++) {
        Start-Sleep -Milliseconds 100
        $button=[MissingAssetProbe]::GetDlgItem($dialog,1)
        if($button -eq [IntPtr]::Zero){$button=[MissingAssetProbe]::GetDlgItem($dialog,2)}
    }
    if($button -eq [IntPtr]::Zero){throw "Startup error OK control missing: $([MissingAssetProbe]::Children($dialog))"}
    [void][MissingAssetProbe]::SetForegroundWindow($dialog)
    [void][MissingAssetProbe]::SendMessage($button,0xf5,[IntPtr]::Zero,[IntPtr]::Zero)
    [void][MissingAssetProbe]::SendMessage($dialog,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
    $finished=$process.WaitForExit(10000)
    if(!$finished -or $process.ExitCode -ne 1){throw "Missing-assets startup expected exit 1: finished=$finished exit=$($process.ExitCode)"}
    $log=Join-Path $env:LOCALAPPDATA "ProjectNera/logs/nera-$($process.Id).log"
    if(![IO.File]::ReadAllText($log).Contains('Required runtime files are missing')){throw 'Startup diagnostic missing.'}
    Copy-Item -LiteralPath $log -Destination (Join-Path (Split-Path -Parent $exe) 'startup-error.log')
    'PASSED: incomplete package shows native dialog, writes actionable log, exits 1.'
} finally {
    if(!$process.HasExited){$process.Kill();$process.WaitForExit()}
    $process.Dispose()
}
