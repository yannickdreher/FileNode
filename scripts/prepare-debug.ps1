$ErrorActionPreference = 'Stop'
$driveProject = Split-Path -Parent $PSScriptRoot
$driveDebug = Join-Path $driveProject '.dev'
$driveProfile = Join-Path $driveDebug 'profile.toml'
New-Item -ItemType Directory -Force (Join-Path $driveDebug 'files') | Out-Null
if (-not (Test-Path -LiteralPath $driveProfile)) {
    $driveLocal = (Join-Path $driveDebug 'files').Replace('\', '/')
    if ($driveLocal.Contains("'")) { throw 'Choose a workspace path without apostrophes for the generated debug profile.' }
    $driveContent = @"
language = "en"
server = "https://jmap.example.com"
web_url = "https://mail.example.com"
username = ""
local_folder = '$driveLocal'
remote_folder = "Noritec Drive Debug"
propagate_deletes = false
auto_connect = false
autostart = false
"@
    [System.IO.File]::WriteAllText($driveProfile, $driveContent, [System.Text.UTF8Encoding]::new($false))
}
Write-Output "Debug profile: $driveProfile"
