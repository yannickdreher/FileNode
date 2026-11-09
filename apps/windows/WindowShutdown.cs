using Microsoft.UI.Xaml;
using System.Runtime.InteropServices;

namespace FileNode.Windows;

// Windows Restart Manager uses session messages, not the user's close-to-tray action.
internal sealed class WindowShutdown : IDisposable
{
    private readonly nint handle;
    private readonly SubclassProc callback;
    private readonly Action shutdown;
    private bool disposed;
    private delegate nint SubclassProc(nint hwnd, uint message, nuint wparam, nint lparam, nuint id, nuint data);

    public WindowShutdown(Window window, Action shutdown)
    {
        handle = WinRT.Interop.WindowNative.GetWindowHandle(window);
        this.shutdown = shutdown;
        callback = Message;
        if (!SetWindowSubclass(handle, callback, 2, 0))
            throw new InvalidOperationException("Unable to subscribe to Windows shutdown messages");
        window.Closed += (_, _) => Dispose();
    }

    private nint Message(nint hwnd, uint message, nuint wparam, nint lparam, nuint id, nuint data)
    {
        if (message == 0x0011) return 1; // WM_QUERYENDSESSION: consent, but do not exit yet.
        if (message == 0x0016) // WM_ENDSESSION: a canceled update/logoff must leave us running.
        {
            if (wparam != 0) shutdown();
            return 0;
        }
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        _ = RemoveWindowSubclass(handle, callback, 2);
    }

    [DllImport("comctl32.dll")] private static extern bool SetWindowSubclass(nint hwnd, SubclassProc proc, nuint id, nuint data);
    [DllImport("comctl32.dll")] private static extern bool RemoveWindowSubclass(nint hwnd, SubclassProc proc, nuint id);
    [DllImport("comctl32.dll")] private static extern nint DefSubclassProc(nint hwnd, uint message, nuint wparam, nint lparam);
}
