param([string]$Executable='target/debug/nera.exe', [string]$Output='target/final-audit/smokes', [string[]]$Only=@(), [switch]$Measure)
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath $Executable).Path
$folder=[IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Path $folder -Force | Out-Null
$cases=@(
 @{name='chibi';args='--feature-smoke --chibi-smoke';timeout=210},
 @{name='cloud';args='--feature-smoke --cloud-smoke';timeout=90},
 @{name='presentation';args='--feature-smoke --presentation-smoke';timeout=180},
 @{name='focus';args='--feature-smoke --focus-activity-smoke';timeout=180},
 @{name='treat';args='--feature-smoke --treat-focus-smoke';timeout=180},
 @{name='touch';args='--feature-smoke --touch-smoke';timeout=120},
 @{name='library-motion';args='--feature-smoke --library-motion-smoke';timeout=180},
 @{name='companions';args='--feature-smoke';timeout=90},
 @{name='settings-memory';args='--character-instance 90651 --settings-memory-audit';timeout=160},
 @{name='alarm-memory';args='--character-instance 90652 --alarm-memory-audit';timeout=210},
 @{name='manager-host';args='--character-instance 90653';timeout=150;environment=@{NERA_GALLERY_HOST_SMOKE_TEST='1'}}
)
if($Only.Count){$cases=@($cases | Where-Object {$_.name -in $Only})}
$results=@()
foreach($case in $cases) {
    $start=[Diagnostics.ProcessStartInfo]::new($exe,$case.args)
    $start.WorkingDirectory=(Get-Location).Path
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true
    $start.RedirectStandardInput=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $start.EnvironmentVariables['NERA_INSTANCE_CHARACTER']='vrm/nera_256.vrm'
    $start.EnvironmentVariables['NERA_INSTANCE_SETTINGS']='{"fps":24,"muted":true,"walking":false,"gravity":true,"x":500,"y":150}'
    if($case.environment){foreach($key in $case.environment.Keys){$start.EnvironmentVariables[$key]=$case.environment[$key]}}
    $process=[Diagnostics.Process]::Start($start)
    $out=$process.StandardOutput.ReadToEndAsync();$err=$process.StandardError.ReadToEndAsync()
    $time=Get-Date
    $measurements=@()
    if($Measure) {
        $finished=$false
        while(((Get-Date)-$time).TotalSeconds -lt $case.timeout) {
            if($process.WaitForExit(1000)){$finished=$true;break}
            $process.Refresh()
            $measurements+=@{utc=(Get-Date).ToUniversalTime().ToString('o');seconds=((Get-Date)-$time).TotalSeconds;private_mib=$process.PrivateMemorySize64/1MB;working_mib=$process.WorkingSet64/1MB;cpu_seconds=$process.TotalProcessorTime.TotalSeconds}
        }
        $measurements | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $folder ($case.name+'.memory.json')) -Encoding utf8
    } else {$finished=$process.WaitForExit($case.timeout*1000)}
    if(!$finished){$process.Kill();$process.WaitForExit()}
    $out.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $folder ($case.name+'.stdout.log')) -Encoding utf8
    $err.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $folder ($case.name+'.stderr.log')) -Encoding utf8
    $status=if($finished -and $process.ExitCode -eq 0){'PASSED'}else{'FAILED'}
    $results+=@{name=$case.name;status=$status;pid=$process.Id;exit_code=$process.ExitCode;seconds=((Get-Date)-$time).TotalSeconds;timed_out=!$finished}
    $results | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $folder 'results.json') -Encoding utf8
    Write-Output "$($case.name): $status exit=$($process.ExitCode)"
    $process.Dispose()
}
if(@($results | Where-Object {$_.status -eq 'FAILED'}).Count){exit 1}
