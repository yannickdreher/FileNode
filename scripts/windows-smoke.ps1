param(
    [string]$Executable = '',
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$driveProject = Split-Path -Parent $PSScriptRoot
if (!$Executable) { $Executable = Join-Path $driveProject '.dev\filenode-preview\filenode-ui.exe' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $driveProject 'artifacts\windows-smoke' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
if ($OutputDirectory.Contains('"')) { throw 'Smoke-test paths must not contain quotation marks.' }
$driveProcess = Start-Process -FilePath $Executable -ArgumentList @('--self-test', ('"' + $OutputDirectory + '"')) -WorkingDirectory $driveProject -WindowStyle Hidden -PassThru
if (!$driveProcess.WaitForExit(30000)) {
    # Stop only the isolated test process created above, never a user's instance.
    $driveProcess.Kill()
    throw 'The isolated Windows UI test timed out.'
}
$driveReportPath = Join-Path $OutputDirectory 'ui-smoke.json'
if (!(Test-Path -LiteralPath $driveReportPath)) { throw "The UI exited without a smoke-test report (exit $($driveProcess.ExitCode))." }
$driveReport = Get-Content -Raw -LiteralPath $driveReportPath | ConvertFrom-Json
if ($driveProcess.ExitCode -ne 0 -or !$driveReport.passed) {
    throw "Windows UI smoke test failed (exit $($driveProcess.ExitCode)): $($driveReport.message)"
}
Write-Output 'Windows UI smoke passed: English/German, verified account vs sync pause, independent folder states, one row per file with progress, multiple folders, stop without deletion, offline, clean quit.'
