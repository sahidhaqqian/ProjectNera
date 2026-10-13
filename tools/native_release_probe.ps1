param([Parameter(Mandatory=$true)][string]$Executable, [string]$Output='target/final-audit/native-release', [int]$Seconds=180, [switch]$Isolated, [switch]$UIWindows, [switch]$CorruptConfig)
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath $Executable).Path
$folder=[IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Path $folder -Force | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Text; using System.Collections.Generic; using System.Runtime.InteropServices;
public static class NeraProbe {
 public delegate bool EnumProc(IntPtr hwnd,IntPtr value);
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback,IntPtr value);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd,StringBuilder text,int count);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd,StringBuilder text,int count);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd,uint message,IntPtr wp,IntPtr lp);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd,uint message,IntPtr wp,IntPtr lp);
 [DllImport("shell32.dll",CharSet=CharSet.Unicode)] public static extern uint ExtractIconEx(string path,int index,out IntPtr large,out IntPtr small,uint count);
 [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr icon);
 public class Window { public long hwnd; public uint pid; public string title; public string cls; public bool visible; }
 public static Window[] Windows(uint process) {
   var result=new List<Window>();
   EnumWindows((h,v)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==process){var t=new StringBuilder(512);var c=new StringBuilder(128);GetWindowText(h,t,t.Capacity);GetClassName(h,c,c.Capacity);result.Add(new Window{hwnd=h.ToInt64(),pid=id,title=t.ToString(),cls=c.ToString(),visible=IsWindowVisible(h)});}return true;},IntPtr.Zero);
   return result.ToArray();
 }
}
'@
$before=@(Get-Process -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
$started=Get-Date
if($CorruptConfig) {
    $Isolated=$true
    $testFolder=Join-Path $env:LOCALAPPDATA 'io.github.sahidhaqqi.projectnera.instance.90613'
    New-Item -ItemType Directory -Path $testFolder -Force | Out-Null
    $testConfig=Join-Path $testFolder 'settings.toml'
    $originalConfig=if(Test-Path -LiteralPath $testConfig){[IO.File]::ReadAllBytes($testConfig)}else{$null}
    [IO.File]::WriteAllText($testConfig,'[ deliberately invalid TOML')
}
if ($Isolated) {
    $launch=[Diagnostics.ProcessStartInfo]::new($exe,'--character-instance 90613')
    $launch.WorkingDirectory=Split-Path -Parent $exe
    $launch.UseShellExecute=$false;$launch.CreateNoWindow=$true
    # Keep the companion owner pipe alive without changing the user's settings.
    $launch.RedirectStandardInput=$true
    $launch.RedirectStandardOutput=$true;$launch.RedirectStandardError=$true
    $launch.EnvironmentVariables['NERA_INSTANCE_CHARACTER']='vrm/nera_256.vrm'
    $launch.EnvironmentVariables['NERA_INSTANCE_SETTINGS']='{"fps":24,"muted":true,"walking":false,"gravity":true,"x":600,"y":150}'
    $owned=[Diagnostics.Process]::Start($launch)
    $capturedOut=$owned.StandardOutput.ReadToEndAsync();$capturedErr=$owned.StandardError.ReadToEndAsync()
} else {
    $shell=New-Object -ComObject Shell.Application
    # Explorer's default open verb. No command shell is used to launch Nera.
    $shell.ShellExecute($exe,'',(Split-Path -Parent $exe),'open',1)
}
$process=$null; $windows=@(); $ready=$null
try {
for($i=0;$i -lt 240;$i++) {
    $process=Get-Process -ErrorAction SilentlyContinue | Where-Object {$_.Id -notin $before -and $_.Path -eq $exe} | Select-Object -First 1
    if($process) {
        $windows=@([NeraProbe]::Windows($process.Id))
        if(@($windows | Where-Object {$_.visible -and $_.title -eq 'Nera'}).Count) {$ready=((Get-Date)-$started).TotalSeconds;break}
    }
    Start-Sleep -Milliseconds 250
}
if(!$process -or $null -eq $ready) { throw 'Shell launch did not create the visible Nera window.' }
$large=[IntPtr]::Zero; $small=[IntPtr]::Zero
if([NeraProbe]::ExtractIconEx($exe,0,[ref]$large,[ref]$small,1) -lt 1 -and !$Isolated) {throw 'Embedded icon extraction failed.'}
try {
    foreach($pair in @(@('exe-large',$large),@('exe-small',$small))) {
        if($pair[1] -eq [IntPtr]::Zero){continue};$icon=[Drawing.Icon]::FromHandle($pair[1]);$bitmap=$icon.ToBitmap()
        try {$bitmap.Save((Join-Path $folder ($pair[0]+'.png')))} finally {$bitmap.Dispose()}
    }
} finally { [void][NeraProbe]::DestroyIcon($large);[void][NeraProbe]::DestroyIcon($small) }
$window=@($windows | Where-Object {$_.title -eq 'Nera'})[0]
$console = python (Join-Path $PSScriptRoot 'check_console.py') $process.Id | ConvertFrom-Json
if (!$Isolated -and ($console.has_console -or $console.win32_error -ne 6)) {throw 'GUI shell launch console check failed.'}
$icons=@()
foreach($size in @(0,1)) {
    $handle=[NeraProbe]::SendMessage([IntPtr]$window.hwnd,0x7f,[IntPtr]$size,[IntPtr]::Zero)
    if(!$Isolated) {
        for($attempt=0;$handle -eq [IntPtr]::Zero -and $attempt -lt 80;$attempt++) {
            Start-Sleep -Milliseconds 250
            $handle=[NeraProbe]::SendMessage([IntPtr]$window.hwnd,0x7f,[IntPtr]$size,[IntPtr]::Zero)
        }
    }
    if($handle -eq [IntPtr]::Zero) {if($Isolated){continue};throw 'Nera window icon is missing.'}
    $icon=[Drawing.Icon]::FromHandle($handle);$bitmap=$icon.ToBitmap()
    try {$bitmap.Save((Join-Path $folder "window-icon-$size.png"));$icons+=@{slot=$size;width=$bitmap.Width;height=$bitmap.Height}} finally {$bitmap.Dispose()}
}
$ui=@()
if($UIWindows) {
    $tray=@([NeraProbe]::Windows($process.Id) | Where-Object {$_.cls -eq 'NeraTrayWindowClass'})[0]
    foreach($item in @(@(1,'Nera Settings'),@(3,'Nera Alarms'),@(2,'Animation Manager'))) {
        [void][NeraProbe]::SendMessage([IntPtr]$tray.hwnd,6003,[IntPtr]600,[IntPtr]300)
        $menu=@([NeraProbe]::Windows($process.Id) | Where-Object {$_.cls -eq 'NeraRadialMenuWindowClass'})[0]
        $angle=-[Math]::PI/2+2*[Math]::PI*$item[0]/7
        $x=[int][Math]::Round(195+139*[Math]::Cos($angle));$y=[int][Math]::Round(195+139*[Math]::Sin($angle))
        [void][NeraProbe]::SendMessage([IntPtr]$menu.hwnd,0x202,[IntPtr]::Zero,[IntPtr](($y -shl 16) -bor $x))
        $target=$null;$worker=$null
        for($i=0;$i -lt 60;$i++) {
            Start-Sleep -Milliseconds 250
            $target=@([NeraProbe]::Windows($process.Id) | Where-Object {$_.visible -and $_.title -eq $item[1]}) | Select-Object -First 1
            if($item[0] -eq 2) {
                $child=Get-CimInstance Win32_Process -Filter "ParentProcessId=$($process.Id)" | Where-Object {$_.ExecutablePath -eq $exe} | Select-Object -First 1
                if($child){$worker=Get-Process -Id $child.ProcessId;$target=@([NeraProbe]::Windows($worker.Id) | Where-Object {$_.visible -and $_.title -eq 'Nera Animations'}) | Select-Object -First 1}
            }
            if($target){break}
        }
        if(!$target){throw "Native radial entry failed: $($item[1])"}
        Start-Sleep -Seconds 1
        $smallIcon=[NeraProbe]::SendMessage([IntPtr]$target.hwnd,0x7f,[IntPtr]::Zero,[IntPtr]::Zero)
        if($smallIcon -eq [IntPtr]::Zero){throw 'Utility window lacks official icon.'}
        $bitmap=[Drawing.Icon]::FromHandle($smallIcon).ToBitmap()
        try{$bitmap.Save((Join-Path $folder ("utility-$($item[0])-icon.png")))}finally{$bitmap.Dispose()}
        $check=python (Join-Path $PSScriptRoot 'check_console.py') $target.pid | ConvertFrom-Json
        if($check.has_console -or $check.win32_error -ne 6){throw 'Utility/worker process opened a console.'}
        $ui+=@{entry=$item[1];window=$target;console=$check;icon='PASSED'}
        [void][NeraProbe]::PostMessage([IntPtr]$target.hwnd,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
        if($worker -and !$worker.WaitForExit(15000)){throw 'Manager worker did not exit on close.'}
        Start-Sleep -Seconds 2
    }
}
$samples=@();$cpu=$process.TotalProcessorTime.TotalSeconds;$last=Get-Date
for($i=0;$i -lt [Math]::Ceiling($Seconds/2);$i++) {
    Start-Sleep -Seconds 2
    $process.Refresh();if($process.HasExited){throw 'Nera exited during soak.'}
    $now=Get-Date;$current=$process.TotalProcessorTime.TotalSeconds
    $samples+=@{seconds=($now-$started).TotalSeconds;private_mib=$process.PrivateMemorySize64/1MB;working_mib=$process.WorkingSet64/1MB;cpu_one_core_percent=100*($current-$cpu)/($now-$last).TotalSeconds}
    $cpu=$current;$last=$now
}
$log=Join-Path $env:LOCALAPPDATA "ProjectNera/logs/nera-$($process.Id).log"
$logText=''
if(Test-Path -LiteralPath $log) {
    $stream=[IO.File]::Open($log,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    $reader=[IO.StreamReader]::new($stream)
    try{$logText=$reader.ReadToEnd()}finally{$reader.Dispose()}
}
if($CorruptConfig -and !$logText.Contains('Error parsing settings file')) {throw 'Corrupt-config recovery did not preserve its diagnostic.'}
$tray=@($windows | Where-Object {$_.cls -match 'Nera.*Tray|Tray.*Nera'})
$steady=@($samples | Where-Object {$_.seconds -ge 25})
$gpu=@()
try {
    $gpu=@((Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples | Where-Object {$_.InstanceName -match "^pid_$($process.Id)_"} | Select-Object InstanceName,CookedValue)
} catch { $gpu=@(@{status='NOT MEASURABLE';reason=$_.Exception.Message}) }
if(!$gpu.Count){$gpu=@(@{status='NOT MEASURABLE';reason='No matching process GPU counter samples.'})}
$runtime=@($process.Modules | Where-Object {$_.ModuleName -match '^(VCRUNTIME|MSVCP).*\.dll$'} | Select-Object ModuleName,FileName)
[ordered]@{executable=$exe;sha256=(Get-FileHash -LiteralPath $exe).Hash;pid=$process.Id;shell_open=$(if($Isolated){'NOT PERFORMED (isolated benchmark)'}else{'PASSED'});profile=$(if($Isolated){'bundled Nera; 24 FPS; muted; roaming off'}else{'saved user preferences'});physical_double_click='NOT PERFORMED';window_seconds=$ready;windows=$windows;icons=$icons;utility_windows=$ui;console_check=$console;console_window_found=[bool]($windows | Where-Object {$_.cls -eq 'ConsoleWindowClass'});tray_registered=$logText.Contains('Tray native icon registered');local_log=[bool]$logText;steady_private_mib=($steady.private_mib | Measure-Object -Average).Average;steady_working_mib=($steady.working_mib | Measure-Object -Average).Average;steady_cpu_one_core_percent=($steady.cpu_one_core_percent | Measure-Object -Average).Average;gpu_engine_snapshot=$gpu;loaded_crt=$runtime;samples=$samples;version=[Diagnostics.FileVersionInfo]::GetVersionInfo($exe).FileVersion} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $folder 'report.json') -Encoding utf8
# Close only this test instance through its normal native close request.
[void][NeraProbe]::PostMessage([IntPtr]$window.hwnd,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
if(!$process.WaitForExit(15000)){Stop-Process -Id $process.Id;throw 'Normal native close did not exit.'}
if(Test-Path -LiteralPath $log){Copy-Item -LiteralPath $log -Destination (Join-Path $folder 'process.log')}
Write-Output "Native probe completed; measured results: $folder/report.json"
} finally {
    if($process -and !$process.HasExited){Stop-Process -Id $process.Id -ErrorAction SilentlyContinue;[void]$process.WaitForExit(5000)}
    if($owned){
        if($owned.HasExited){
            $capturedOut.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $folder 'stdout.log') -Encoding utf8
            $capturedErr.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $folder 'stderr.log') -Encoding utf8
        }
        $owned.Dispose()
    }
    if($CorruptConfig) {
        if($null -ne $originalConfig){[IO.File]::WriteAllBytes($testConfig,$originalConfig)}
        else{Remove-Item -LiteralPath $testConfig -ErrorAction SilentlyContinue}
    }
}
