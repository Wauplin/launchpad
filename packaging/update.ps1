param([switch]$DownloadOnly, [string]$DataDir = (Join-Path $HOME '.local/share/launchpad'))
$ErrorActionPreference = 'Stop'
$env:LAUNCHPAD_PROFILE = 'prod'
$Data = $DataDir
New-Item -ItemType Directory -Force $Data | Out-Null
$MutexName = if ($DownloadOnly) { 'Local\LaunchpadUpdateDownload' } else { 'Local\LaunchpadUpdate' }
$Mutex = New-Object Threading.Mutex($false, $MutexName)
if (-not $Mutex.WaitOne(0)) { exit 0 }
$Reserved = $false
$Cli = if ($env:LAUNCHPAD_BIN_DIR) { Join-Path $env:LAUNCHPAD_BIN_DIR 'launchpad.exe' } else { Join-Path $HOME '.local/bin/launchpad.exe' }
function Status($State, $Stage, $Message) { [IO.File]::WriteAllText((Join-Path $Data 'update-status'), "$State`t$Stage`t$Message") }
function Check { if ($LASTEXITCODE -ne 0) { throw "Command failed with exit code $LASTEXITCODE" } }
Start-Transcript -Path (Join-Path $Data 'update.log') -Force | Out-Null
try {
    Status running 'Downloading GitHub build' ''
    $Repo = 'Wauplin/launchpad'
    $Revision = gh api "repos/$Repo/commits/main" --jq .sha; Check
    $RunsJson = gh api "repos/$Repo/actions/workflows/ci.yml/runs?branch=main&head_sha=$Revision&status=success&per_page=100"; Check
    $Run = (($RunsJson | ConvertFrom-Json).workflow_runs | Where-Object { $_.event -in @('push', 'workflow_dispatch') } | Select-Object -First 1).id
    if (-not $Run) { throw "GitHub CI has not passed for latest main ($Revision). Retry when it finishes." }
    $Work = Join-Path $Data ([guid]::NewGuid().ToString())
    New-Item -ItemType Directory $Work | Out-Null
    gh run download $Run --repo $Repo --name launchpad-windows-x86_64 --dir $Work; Check
    $Archive = Join-Path $Work 'launchpad-windows-x86_64.tar.gz'
    $Expected = (Get-Content "$Archive.sha256").Split(' ')[0]
    if ((Get-FileHash $Archive -Algorithm SHA256).Hash -ne $Expected) { throw 'Artifact checksum mismatch' }
    $Release = Join-Path $Work 'release'
    New-Item -ItemType Directory $Release | Out-Null
    tar -xzf $Archive -C $Release; Check
    if ((Get-Content (Join-Path $Release 'COMMIT')) -ne $Revision) { throw 'Artifact commit mismatch' }
    if ($DownloadOnly) { Status complete 'Download verified' "Verified GitHub build $Revision for windows-x86_64; installation skipped."; return }
    Status running 'Waiting for agents to stop' ''
    while ($true) {
        & $Cli update-install-lock
        if ($LASTEXITCODE -eq 0) { $Reserved = $true; break }
        if ($LASTEXITCODE -ne 3) { throw 'Could not reserve idle daemon' }
        Start-Sleep -Seconds 2
    }
    Status running 'Installing and relaunching' ''
    & (Join-Path $Release 'packaging/install.ps1') -Prebuilt -BinDir $env:LAUNCHPAD_BIN_DIR
    $Reserved = $false
    Status complete Complete 'Launchpad is up to date.'
} catch { Status failed Failed $_.Exception.Message; throw }
finally {
    if ($Reserved) { & $Cli update-install-lock --release }
    Set-Location $Data
    if ($Work -and (Test-Path $Work)) { Remove-Item $Work -Recurse -Force }
    Stop-Transcript | Out-Null
    $Mutex.ReleaseMutex(); $Mutex.Dispose()
}
