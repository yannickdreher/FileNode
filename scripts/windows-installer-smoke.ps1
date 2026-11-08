param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Version
)
$ErrorActionPreference = 'Stop'
# Install/uninstall only on a disposable hosted runner, never a user's desktop.
if ($env:GITHUB_ACTIONS -ne 'true' -or !$env:RUNNER_TEMP) { throw 'Installer smoke tests require an isolated GitHub Actions runner.' }
$driveSetup = (Resolve-Path -LiteralPath $Installer).Path
$driveTempRoot = [System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
$driveInstallRoot = [System.IO.Path]::GetFullPath((Join-Path $driveTempRoot ('filenode-installer-' + [guid]::NewGuid())))
if (!$driveInstallRoot.StartsWith($driveTempRoot, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected installer test directory.' }
$driveRegistration = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{263F7D9C-62D7-4648-93B8-71B2DB01D376}_is1'
$driveShortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'FileNode.lnk'
if ((Test-Path -LiteralPath $driveRegistration) -or (Test-Path -LiteralPath $driveShortcut)) { throw 'An existing FileNode installation must not be modified by a smoke test.' }
$driveUninstalled = $false
try {
    $driveProcess = Start-Process -FilePath $driveSetup -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOCLOSEAPPLICATIONS', ('/DIR="' + $driveInstallRoot + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($driveProcess.ExitCode -ne 0) { throw "Installer failed with exit code $($driveProcess.ExitCode)." }
    foreach ($driveFile in @('filenode-ui.exe', 'filenode-ui.dll', 'filenode-core.exe', 'locales/en.json', 'locales/de.json', 'Assets/filenode.ico')) {
        if (!(Test-Path -LiteralPath (Join-Path $driveInstallRoot $driveFile))) { throw "Installer omitted $driveFile" }
    }
    if (!(Test-Path -LiteralPath $driveShortcut)) { throw 'Start menu shortcut was not created.' }
    if ((Get-ItemProperty -LiteralPath $driveRegistration).DisplayVersion -ne $Version) { throw 'Installed version does not match the release.' }
    $driveActual = & (Join-Path $driveInstallRoot 'filenode-core.exe') --version
    if ($LASTEXITCODE -ne 0 -or $driveActual -ne "filenode $Version") { throw 'Installed worker has an incorrect version.' }
    # An uninstaller must remove only its registered payload, not user-created files.
    Set-Content -LiteralPath (Join-Path $driveInstallRoot 'keep.txt') -Value 'Unowned smoke fixture'
    $driveProcess = Start-Process -FilePath (Join-Path $driveInstallRoot 'unins000.exe') -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -WindowStyle Hidden -Wait -PassThru
    if ($driveProcess.ExitCode -ne 0) { throw "Uninstaller failed with exit code $($driveProcess.ExitCode)." }
    $driveDeadline = (Get-Date).AddSeconds(15)
    while (((Test-Path -LiteralPath $driveRegistration) -or (Test-Path -LiteralPath $driveShortcut) -or (Test-Path -LiteralPath (Join-Path $driveInstallRoot 'filenode-core.exe')) -or (Test-Path -LiteralPath (Join-Path $driveInstallRoot 'unins000.exe'))) -and (Get-Date) -lt $driveDeadline) { Start-Sleep -Milliseconds 200 }
    if ((Test-Path -LiteralPath $driveRegistration) -or (Test-Path -LiteralPath $driveShortcut) -or (Test-Path -LiteralPath (Join-Path $driveInstallRoot 'filenode-core.exe'))) { throw 'Uninstaller left registered files or shortcuts.' }
    if (!(Test-Path -LiteralPath (Join-Path $driveInstallRoot 'keep.txt'))) { throw 'Uninstaller removed an unowned file.' }
    $driveUninstalled = $true
    Write-Output 'Windows installer smoke passed: install, payload, Start menu, version, uninstall, unowned file preserved; no account or UI launched.'
} finally {
    # Never recursively remove a computed target outside the verified runner temp root.
    if ($driveUninstalled -and $driveInstallRoot.StartsWith($driveTempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $driveInstallRoot -Recurse -Force
    }
}
