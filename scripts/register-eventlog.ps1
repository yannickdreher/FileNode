#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
# The Windows-supplied message resource renders one insertion string for IDs
# 1..1000. No application DLL or user-writable executable path is registered.
$sourcePath = 'HKLM:\SYSTEM\CurrentControlSet\Services\EventLog\Application\FileNode'
if (Test-Path -LiteralPath $sourcePath) {
    throw 'The FileNode event source already exists. Its configuration was not changed.'
}
New-Item -Path $sourcePath -Force | Out-Null
New-ItemProperty -LiteralPath $sourcePath -Name EventMessageFile -PropertyType ExpandString -Value '%SystemRoot%\System32\EventCreate.exe' | Out-Null
New-ItemProperty -LiteralPath $sourcePath -Name TypesSupported -PropertyType DWord -Value 7 | Out-Null
Write-Output 'Registered FileNode in Windows Logs / Application. Restart FileNode to use the registered message resource.'
