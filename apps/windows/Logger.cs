using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.Win32;

namespace FileNode.Windows;

internal static class Logger
{
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern nint RegisterEventSource(string? server, string source);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ReportEvent(nint source, ushort type, ushort category, uint id, nint sid,
        ushort count, uint size, [MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.LPWStr)] string[] strings, nint data);
    [DllImport("advapi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DeregisterEventSource(nint source);

    internal static void Unhandled(Exception exception)
    {
        if (App.SelfTestDirectory is not null || Environment.GetEnvironmentVariable("FILENODE_LOG") == "off") return;
        nint source = 0;
        try
        {
            using var key = Registry.LocalMachine.OpenSubKey(@"SYSTEM\CurrentControlSet\Services\EventLog\Application\FileNode");
            var registered = string.Equals(key?.GetValue("EventMessageFile") as string,
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "System32", "EventCreate.exe"), StringComparison.OrdinalIgnoreCase);
            // Log method names, exception type and HRESULT; omit exception messages,
            // arguments and source paths which may include credentials or account URLs.
            var methods = string.Join(" <- ", new StackTrace(exception).GetFrames().Take(20)
                .Select(frame => frame.GetMethod()).Where(method => method is not null)
                .Select(method => method!.DeclaringType?.FullName + "." + method.Name));
            var text = $"FileNode event=ui_unhandled_exception pid={Environment.ProcessId} exception={exception.GetType().FullName} hresult=0x{exception.HResult:X8} methods={methods}";
            source = RegisterEventSource(null, "FileNode");
            if (source != 0) _ = ReportEvent(source, 1, 0, registered ? 398u : 0x2000018Eu, 0, 1, 0, [text], 0);
        }
        catch { /* Logging must not replace the original exception or trigger a second crash. */ }
        finally { if (source != 0) _ = DeregisterEventSource(source); }
    }
}
