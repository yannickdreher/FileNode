param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = 'artifacts/windows-shutdown',
    [string]$Installer = ''
)
$ErrorActionPreference = 'Stop'
$driveExecutable = (Resolve-Path -LiteralPath $Executable).Path
$driveOutput = [IO.Path]::GetFullPath($OutputDirectory)
if ($driveOutput.Contains('"')) { throw 'Test paths must not contain quotation marks.' }
if ($Installer) {
    if ($env:GITHUB_ACTIONS -ne 'true' -or !$env:RUNNER_TEMP) { throw 'Installer upgrade tests require a disposable GitHub Actions runner.' }
    $driveRunnerRoot = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
    if (!$driveExecutable.StartsWith($driveRunnerRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Installer upgrade fixture must be inside the runner temp directory.' }
    $Installer = (Resolve-Path -LiteralPath $Installer).Path
}
if (!('FileNodeShutdownMessages' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FileNodeShutdownMessages {
    [StructLayout(LayoutKind.Sequential)] private struct UniqueProcess { public uint Id; public System.Runtime.InteropServices.ComTypes.FILETIME Start; }
    [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SendMessageTimeout(IntPtr hwnd,uint message,UIntPtr wparam,IntPtr lparam,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint process);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr hwnd,int mode);
    [DllImport("rstrtmgr.dll",CharSet=CharSet.Unicode)] private static extern int RmStartSession(out uint session,uint flags,StringBuilder key);
    [DllImport("rstrtmgr.dll",CharSet=CharSet.Unicode)] private static extern int RmRegisterResources(uint session,uint files,string[] names,uint processes,UniqueProcess[] apps,uint services,string[] serviceNames);
    [DllImport("rstrtmgr.dll")] private static extern int RmShutdown(uint session,uint flags,IntPtr callback);
    [DllImport("rstrtmgr.dll")] private static extern int RmEndSession(uint session);
    private static void Verify(long hwnd,int owner) {
        uint actual; GetWindowThreadProcessId(new IntPtr(hwnd),out actual);
        if(actual != (uint)owner) throw new InvalidOperationException("Refusing to control a window outside the isolated test process");
    }
    public static void Hide(long hwnd,int owner) { Verify(hwnd,owner); ShowWindow(new IntPtr(hwnd),0); }
    public static void RestartManager(int owner,long started) {
        uint session;
        var result=RmStartSession(out session,0,new StringBuilder(33));
        if(result!=0) throw new System.ComponentModel.Win32Exception(result);
        try {
            var app=new UniqueProcess {Id=(uint)owner,Start=new System.Runtime.InteropServices.ComTypes.FILETIME {dwLowDateTime=unchecked((int)started),dwHighDateTime=(int)(started>>32)}};
            result=RmRegisterResources(session,0,null,1,new[]{app},0,null);
            if(result!=0) throw new System.ComponentModel.Win32Exception(result);
            result=RmShutdown(session,0,IntPtr.Zero);
            if(result!=0) throw new System.ComponentModel.Win32Exception(result);
        } finally { RmEndSession(session); }
    }
    public static ulong Send(long hwnd,int owner,uint message,bool end) {
        Verify(hwnd,owner);
        UIntPtr result;
        if(SendMessageTimeout(new IntPtr(hwnd),message,new UIntPtr(end ? 1u : 0u),new IntPtr(1),2,12000,out result)==IntPtr.Zero)
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return result.ToUInt64();
    }
}
'@
}
$driveCases = @('status', 'settings', 'restart-manager')
if ($Installer) { $driveCases += 'installer' }
foreach ($driveCase in $driveCases) {
    $driveFixture = Join-Path $driveOutput ($driveCase + '-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $driveFixture -Force | Out-Null
    $driveUi = $null; $driveWorker = $null
    try {
        $driveUi = Start-Process -FilePath $driveExecutable -ArgumentList @('--shutdown-test', ('"' + $driveFixture + '"')) -WindowStyle Hidden -PassThru
        $driveReady = Join-Path $driveFixture 'shutdown-ready.json'
        $driveLimit = (Get-Date).AddSeconds(20)
        while (!(Test-Path -LiteralPath $driveReady) -and !$driveUi.HasExited -and (Get-Date) -lt $driveLimit) { Start-Sleep -Milliseconds 50 }
        if (!(Test-Path -LiteralPath $driveReady)) { throw 'Shutdown fixture did not start; see its diagnostic report.' }
        $driveState = Get-Content -LiteralPath $driveReady -Raw | ConvertFrom-Json
        if ($driveState.ui -ne $driveUi.Id) { throw 'Shutdown fixture process mismatch.' }
        $driveWorker = Get-Process -Id $driveState.worker -ErrorAction Stop
        # Open the handle while it is alive so .NET Framework can retain its exit code.
        $null = $driveWorker.Handle
        foreach ($driveWindow in @($driveState.status, $driveState.settings)) {
            if ([FileNodeShutdownMessages]::Send($driveWindow, $driveUi.Id, 0x11, $false) -ne 1) { throw 'Window vetoed the update request.' }
            [void][FileNodeShutdownMessages]::Send($driveWindow, $driveUi.Id, 0x16, $false)
        }
        Start-Sleep -Milliseconds 100
        if ($driveUi.HasExited -or $driveWorker.HasExited) { throw 'Canceled update terminated FileNode.' }
        if ($driveState.tray) {
            [void][FileNodeShutdownMessages]::Send($driveState.status, $driveUi.Id, 0x10, $false)
            Start-Sleep -Milliseconds 100
            if ($driveUi.HasExited) { throw 'Normal close no longer hides the tray application.' }
        }
        if ($driveCase -eq 'installer' -or $driveCase -eq 'restart-manager') {
            # Real background operation: neither top-level window is visible.
            [FileNodeShutdownMessages]::Hide($driveState.status, $driveUi.Id)
            [FileNodeShutdownMessages]::Hide($driveState.settings, $driveUi.Id)
        }
        if ($driveCase -eq 'restart-manager') {
            [FileNodeShutdownMessages]::RestartManager($driveUi.Id, $driveUi.StartTime.ToUniversalTime().ToFileTimeUtc())
        } elseif ($driveCase -eq 'installer') {
            # Keep the fixture running while the real installer replaces its locked payload.
            $driveInstallRoot = Split-Path -Parent $driveExecutable
            $driveSetup = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CLOSEAPPLICATIONS', ('/DIR="' + $driveInstallRoot + '"'), ('/LOG="' + (Join-Path $driveFixture 'upgrade.log') + '"')) -WindowStyle Hidden -PassThru
            if (!$driveSetup.WaitForExit(60000)) { $driveSetup.Kill(); throw 'Installer upgrade timed out.' }
            if ($driveSetup.ExitCode -ne 0) { throw "Installer upgrade failed with exit code $($driveSetup.ExitCode)." }
        } else {
            # The same notification Windows Restart Manager sends after consent.
            [void][FileNodeShutdownMessages]::Send($driveState.$driveCase, $driveUi.Id, 0x16, $true)
        }
        if (!$driveUi.WaitForExit(15000)) { throw 'Shutdown left the UI running.' }
        if (!$driveWorker.WaitForExit(15000)) { throw 'Shutdown left the Rust worker running.' }
        if ($driveUi.ExitCode -ne 0 -or $driveWorker.ExitCode -ne 0) {
            throw "Shutdown was not clean: UI=$($driveUi.ExitCode), worker=$($driveWorker.ExitCode)."
        }
        if (Test-Path -LiteralPath (Join-Path $driveFixture 'ui-smoke.json')) {
            throw 'The shutdown fixture recorded an exception; see ui-smoke.json.'
        }
        Write-Output "Windows shutdown passed ($driveCase): query accepted, canceled update preserved processes, UI and worker exited cleanly."
    } finally {
        # Only terminate the isolated blank-account fixtures started above.
        if ($driveUi -and !$driveUi.HasExited) { $driveUi.Kill() }
        if ($driveWorker -and !$driveWorker.HasExited) { $driveWorker.Kill() }
        if ($driveUi) { $driveUi.Dispose() }
        if ($driveWorker) { $driveWorker.Dispose() }
    }
}
