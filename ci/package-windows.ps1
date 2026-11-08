$ErrorActionPreference = 'Stop'
if (!$env:RELEASE_VERSION) { Write-Output 'No new release.'; exit 0 }
node ci/stamp-version.mjs $env:RELEASE_VERSION
if ($LASTEXITCODE -ne 0) { throw 'Version stamping failed.' }
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/build-windows.ps1 -Configuration Release -OutputDirectory dist/windows
if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }
Copy-Item -LiteralPath README.md,README.de.md,config.example.toml,.release/RELEASE-NOTES.md -Destination dist/windows
$drivePrefix = 'dist/filenode-' + $env:RELEASE_VERSION + '-windows-x86_64'
Compress-Archive -Path dist/windows/* -DestinationPath ($drivePrefix + '.zip') -Force
$driveReleaseCore = Join-Path (Get-Location).Path 'target/x86_64-pc-windows-msvc/release/filenode.exe'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/build-windows.ps1 -Configuration Release -Package -RustCorePath $driveReleaseCore -OutputDirectory dist/windows-msix
if ($LASTEXITCODE -ne 0) { throw 'Windows MSIX build failed.' }
$driveMsix = @(Get-ChildItem -LiteralPath apps/windows/AppPackages -Recurse -Filter '*.msix' | Where-Object { $_.Name -like ('*_' + $env:RELEASE_VERSION + '.0_x64.msix') })
if ($driveMsix.Count -ne 1) { throw 'Expected exactly one versioned Windows x64 MSIX.' }
Copy-Item -LiteralPath $driveMsix[0].FullName -Destination ($drivePrefix + '.msix')
Copy-Item -LiteralPath target/x86_64-pc-windows-msvc/release/filenode.exe -Destination ($drivePrefix + '-cli.exe')
$driveVersion = & target/x86_64-pc-windows-msvc/release/filenode.exe --version
if ($driveVersion -ne ('filenode ' + $env:RELEASE_VERSION)) { throw 'Windows binary version does not match the release.' }
node ci/release/manifest.mjs $env:RELEASE_VERSION windows
if ($LASTEXITCODE -ne 0) { throw 'Windows build manifest failed.' }
