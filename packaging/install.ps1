param([switch]$Prebuilt, [string]$BinDir = (Join-Path $HOME '.local/bin'))
$ErrorActionPreference = 'Stop'
$env:LAUNCHPAD_PROFILE = 'prod'
Set-Location (Join-Path $PSScriptRoot '..')
function Check { if ($LASTEXITCODE -ne 0) { throw "Command failed with exit code $LASTEXITCODE" } }
if (-not $Prebuilt) {
    Push-Location web
    try { corepack enable; Check; pnpm install --frozen-lockfile; Check; pnpm build; Check } finally { Pop-Location }
    cargo build --release --locked -p launchpad -p launchpad-desktop; Check
}
$Cli = Join-Path $BinDir 'launchpad.exe'
$Desktop = Join-Path $BinDir 'launchpad-desktop.exe'
New-Item -ItemType Directory -Force $BinDir | Out-Null
foreach ($Binary in @('launchpad.exe', 'launchpad-desktop.exe')) { Copy-Item "target/release/$Binary" (Join-Path $BinDir "$Binary.new") -Force }
Copy-Item skills $BinDir -Recurse -Force
if (Test-Path $Desktop) { & $Desktop --quit; Start-Sleep -Seconds 1 }
# Stop only this installation's daemon.
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $Cli -and $_.CommandLine -match '\sdaemon(?:\s|$)' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Start-Sleep -Milliseconds 500
foreach ($Binary in @('launchpad.exe', 'launchpad-desktop.exe')) { Move-Item (Join-Path $BinDir "$Binary.new") (Join-Path $BinDir $Binary) -Force }
# A login shortcut starts the warm desktop, which starts the detached daemon.
$Shell = New-Object -ComObject WScript.Shell
foreach ($Folder in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Startup'))) {
    $Link = $Shell.CreateShortcut((Join-Path $Folder 'Launchpad.lnk'))
    $Link.TargetPath = $Desktop
    if ($Folder -eq [Environment]::GetFolderPath('Startup')) { $Link.Arguments = '--hidden' }
    $Link.Save()
}
$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($UserPath -split ';') -notcontains $BinDir) { [Environment]::SetEnvironmentVariable('Path', "$BinDir;$UserPath", 'User') }
Start-Process $Desktop
Write-Output 'Launchpad installed. Open it from the Start menu.'
