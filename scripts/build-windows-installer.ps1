param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$SourceDirectory = 'dist/windows',
    [string]$OutputDirectory = 'dist',
    [string]$CompilerPath = ''
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') { throw 'Expected a numeric X.Y.Z version.' }
$driveProject = Split-Path -Parent $PSScriptRoot
Push-Location $driveProject
try {
    $driveSource = (Resolve-Path -LiteralPath $SourceDirectory).Path
    foreach ($driveRequired in @('filenode-ui.exe', 'filenode-ui.dll', 'filenode-core.exe', 'locales/en.json', 'locales/de.json', 'Assets/filenode.ico')) {
        if (!(Test-Path -LiteralPath (Join-Path $driveSource $driveRequired) -PathType Leaf)) { throw "Missing desktop payload: $driveRequired" }
    }
    if (!$CompilerPath) {
        $driveCompiler = Get-Command ISCC.exe -CommandType Application -ErrorAction SilentlyContinue
        if ($driveCompiler) { $CompilerPath = $driveCompiler.Source }
        else { $CompilerPath = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe' }
    }
    if (!(Test-Path -LiteralPath $CompilerPath -PathType Leaf)) { throw 'Inno Setup 6.6+ is required to build the Windows installer. See docs/desktop-frontends.md.' }
    $driveOutput = [System.IO.Path]::GetFullPath($OutputDirectory)
    New-Item -ItemType Directory -Path $driveOutput -Force | Out-Null
    & $CompilerPath "/DAppVersion=$Version" "/DSourceDir=$driveSource" "/DOutputDir=$driveOutput" (Join-Path $driveProject 'apps/windows/installer.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer compilation failed.' }
    $driveInstaller = Join-Path $driveOutput "filenode-$Version-windows-x86_64-setup.exe"
    if (!(Test-Path -LiteralPath $driveInstaller -PathType Leaf)) { throw 'The expected setup executable was not produced.' }
    Write-Output $driveInstaller
} finally { Pop-Location }
