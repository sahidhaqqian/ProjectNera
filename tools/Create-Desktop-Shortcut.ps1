param([string]$Destination = [Environment]::GetFolderPath('Desktop'))
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'nera.exe'
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Extract the complete Nera ZIP first.' }
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $Destination 'Project Nera.lnk'))
$shortcut.TargetPath = $exe
$shortcut.WorkingDirectory = $PSScriptRoot
$shortcut.IconLocation = "$exe,0"
$shortcut.Description = 'Project Nera desktop companion'
$shortcut.Save()
