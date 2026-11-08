param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Debug',
    [ValidateSet('x64', 'ARM64')][string]$Platform = 'x64',
    [switch]$Package,
    [string]$RustCorePath = '',
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$driveProject = Split-Path -Parent $PSScriptRoot
Push-Location $driveProject
try {
    if (!(Get-Command dotnet -CommandType Application -ErrorAction SilentlyContinue)) {
        throw 'The .NET 10 SDK is required. Install it, restart VS Code, then run this task again. See docs/development.md.'
    }
    if (!$RustCorePath) {
        if (!(Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue)) {
            throw 'Cargo was not found. Install Rust with rustup (https://rust-lang.org/tools/install/), and add Desktop development with C++ plus the Windows SDK in Visual Studio Installer. Restart VS Code after installation. To test an existing build without Cargo, choose the FileNode: Windows existing build launch profile. See docs/development.md.'
        }
        $driveTarget = if ($Platform -eq 'ARM64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
        $driveCargoArgs = @('build', '--locked', '--target', $driveTarget)
        if ($Configuration -eq 'Release') { $driveCargoArgs += '--release' }
        & cargo @driveCargoArgs
        if ($LASTEXITCODE -ne 0) { throw 'Rust build failed. Install the MSVC Rust toolchain, C++ build tools and Windows SDK.' }
        $driveProfile = $Configuration.ToLowerInvariant()
        $RustCorePath = Join-Path $driveProject "target\$driveTarget\$driveProfile\filenode.exe"
    }
    $RustCorePath = (Resolve-Path -LiteralPath $RustCorePath).Path
    $driveBuildArgs = @('build', 'apps/windows/FileNode.Windows.csproj', '-c', $Configuration, "-p:Platform=$Platform", "-p:RustCorePath=$RustCorePath")
    if ($OutputDirectory) {
        # WinUI concatenates OutputPath and App.xbf. Preserve the final separator.
        $driveOutput = [System.IO.Path]::GetFullPath($OutputDirectory).Replace('\', '/').TrimEnd('/') + '/'
        $driveBuildArgs += "-p:OutputPath=$driveOutput"
    }
    if ($Package) { $driveBuildArgs += @('-p:PackageForStore=true', '-p:GenerateAppxPackageOnBuild=true', "-p:IntermediateOutputPath=obj/msix/$Platform/$Configuration/") }
    & dotnet @driveBuildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Windows frontend build failed.' }
} finally { Pop-Location }
