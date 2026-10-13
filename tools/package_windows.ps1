param([string]$OutputRoot = 'target/distribution')
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repo
$version = ([regex]::Match([IO.File]::ReadAllText('Cargo.toml'), '(?m)^version = "([^"]+)"')).Groups[1].Value
$root = [IO.Path]::GetFullPath((Join-Path $repo $OutputRoot))
if (!$root.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Output must be inside the repository.' }
$name = "Nera-$version-windows-x64"
$stage = Join-Path $root $name
if (Test-Path -LiteralPath $stage) { throw "Package folder already exists: $stage. Use a new OutputRoot." }
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item -LiteralPath 'target/release/nera.exe' -Destination (Join-Path $stage 'nera.exe')
$pe = python tools/inspect_pe.py target/release/nera.exe | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $pe.subsystem -ne 2) { throw 'Not a validated Windows GUI release executable.' }
# App-local redistributable CRT only when imported; never copy DLLs from System32.
$needed = @($pe.dlls | Where-Object {$_ -match '^(VCRUNTIME|MSVCP).*\.dll$'})
if ($needed.Count) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products '*' -property installationPath
    if (!$vs) { throw 'Visual Studio redistributable source not found.' }
    $redist = Join-Path $vs 'VC/Redist/MSVC'
    foreach ($dll in $needed) {
        $source = Get-ChildItem -LiteralPath $redist -Recurse -File -Filter $dll | Where-Object {$_.FullName -match '\\x64\\Microsoft\.VC\d+\.CRT\\' -and $_.FullName -notmatch '\\onecore\\'} | Sort-Object FullName -Descending | Select-Object -First 1
        if (!$source) { throw "Redistributable runtime not found: $dll" }
        $signature = Get-AuthenticodeSignature -LiteralPath $source.FullName
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft') { throw "Invalid Microsoft runtime signature: $dll" }
        Copy-Item -LiteralPath $source.FullName -Destination (Join-Path $stage $dll)
    }
    @'
Microsoft Visual C++ Runtime redistributable files, copied unchanged from the
licensed Visual Studio Redist directory, are Copyright Microsoft Corporation.
Their use/distribution is subject to Microsoft Software License Terms:
https://visualstudio.microsoft.com/license-terms/
Redistribution guidance and distributable-code list:
https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files
These DLLs are independent of the Project Nera proprietary software license.
App-local runtime security updates require repackaging with an updated toolchain.
'@ | Set-Content -LiteralPath (Join-Path $stage 'MICROSOFT-RUNTIME-NOTICE.txt') -Encoding utf8
}
$files = @(git -c core.quotepath=false ls-files --cached --others --exclude-standard -- assets)
if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate distribution assets.' }
foreach ($file in ($files | Sort-Object -Unique)) {
    # Only the default avatar and shipped runtime directories; exclude local collections.
    if ($file -notmatch '^assets/(vrm/nera_256\.vrm|vrm/Cloud/|animations/animasi_basic/|props/|sound/|shaders/|branding/|licenses/)') { continue }
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { continue }
    $target = Join-Path $stage $file
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath $file -Destination $target
}
$required = @('vrm/nera_256.vrm','sound/classic.wav','shaders/nera_shadow_only.wgsl','branding/nera.ico','licenses/DEPENDENCY_LICENSES.txt','licenses/FiraMono-LICENSE.txt')
foreach ($source in Get-ChildItem -LiteralPath 'src' -Filter '*.rs' -Recurse -File) {
    if ($source.Name -match 'test|smoke') { continue }
    $runtime = [IO.File]::ReadAllText($source.FullName).Split(@('#[cfg(test)]'), [StringSplitOptions]::None)[0]
    foreach ($match in [regex]::Matches($runtime, '"((?:animations/animasi_basic/|props/|sound/|vrm/Cloud/)[^"\r\n]+\.(?:vrma|glb|ogg|wav))(?:#[^"]*)?"')) {
        $required += $match.Groups[1].Value
    }
}
foreach ($asset in ($required | Sort-Object -Unique)) {
    $path = Join-Path $stage "assets/$asset"
    if (!(Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0) { throw "Missing runtime asset: $asset" }
}
foreach ($file in Get-ChildItem -LiteralPath (Join-Path $stage 'assets') -Recurse -File) {
    $stream = [IO.File]::OpenRead($file.FullName)
    try { $bytes = New-Object byte[] 128; $length = $stream.Read($bytes,0,128) } finally { $stream.Dispose() }
    if ([Text.Encoding]::ASCII.GetString($bytes,0,$length).StartsWith('version https://git-lfs.github.com/spec/')) { throw "Unresolved Git LFS pointer: $($file.FullName)" }
}
foreach ($doc in @('README.md','LICENSE','THIRD_PARTY_NOTICES.md','FINAL_AUDIT.md','RELEASE_CHECKLIST.md')) { Copy-Item -LiteralPath $doc -Destination $stage }
Copy-Item -LiteralPath 'tools/Create-Desktop-Shortcut.ps1' -Destination $stage
Copy-Item -LiteralPath 'tools/Create-Desktop-Shortcut.cmd' -Destination $stage
@'
Extract this entire ZIP before running nera.exe. Keep nera.exe and assets together.
No Rust or Cargo is required. Right-click Nera for the radial menu; use the tray to
Show, Hide, Reset Position, Screenshot or Exit. Optional shortcut helper:
Create-Desktop-Shortcut.cmd. Logs: %LOCALAPPDATA%\ProjectNera\logs\.
This executable is unsigned; inspect the publisher/source before approving it.
'@ | Set-Content -LiteralPath (Join-Path $stage 'START-HERE.txt') -Encoding utf8
$records = foreach ($file in Get-ChildItem -LiteralPath $stage -Recurse -File) {
    [ordered]@{path=$file.FullName.Substring($stage.Length+1).Replace('\','/'); bytes=$file.Length; sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash}
}
[ordered]@{version=$version; commit=(git rev-parse HEAD); dirty=[bool](git status --porcelain); rust=(rustc --version); built_utc=(Get-Date).ToUniversalTime().ToString('o'); files=@($records)} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $stage 'MANIFEST.json') -Encoding utf8
New-Item -ItemType Directory -Path $root -Force | Out-Null
$zip = Join-Path $root "$name.zip"
Compress-Archive -LiteralPath $stage -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
"$hash  $name.zip" | Set-Content -LiteralPath "$zip.sha256" -Encoding ascii
Write-Output "PACKAGE=$zip"
Write-Output "SHA256=$hash"
