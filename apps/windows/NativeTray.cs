using System.Runtime.InteropServices;

namespace Noritec.NoriDrive.Windows;

// Explorer notification icon and power notifications share the WinUI window's message loop.
internal sealed class NativeTray : IDisposable
{
    private const uint Callback = 0x8000 + 42;
    private readonly nint window;
    private readonly SubclassProc callback;
    private readonly Func<string, string> translate;
    private readonly Action<string> action;
    private readonly Action<bool, bool, bool> power;
    private readonly nint icon;
    private readonly nint powerRegistration;
    private readonly uint taskbarCreated;
    private bool sleeping;
    public bool Available { get; private set; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NotifyData
    {
        public uint Size; public nint Window; public uint Id, Flags, CallbackMessage; public nint Icon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string Tip;
        public uint State, StateMask;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string Info;
        public uint Version;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string InfoTitle;
        public uint InfoFlags; public Guid Guid; public nint BalloonIcon;
    }
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] private struct PowerStatus
    { public byte ACLineStatus, BatteryFlag, BatteryLifePercent, SystemStatusFlag; public uint BatteryLifeTime, BatteryFullLifeTime; }
    private delegate nint SubclassProc(nint hwnd, uint message, nuint wparam, nint lparam, nuint id, nuint data);
    public NativeTray(nint window, Func<string, string> translate, Action<string> action, Action<bool, bool, bool> power)
    {
        this.window = window; this.translate = translate; this.action = action; this.power = power;
        callback = Message;
        if (!SetWindowSubclass(window, callback, 1, 0)) throw new InvalidOperationException("Unable to subscribe to native window messages");
        icon = LoadImage(0, Path.Combine(AppContext.BaseDirectory, "Assets", "drive.ico"), 1, 0, 0, 0x10 | 0x40);
        taskbarCreated = RegisterWindowMessage("TaskbarCreated");
        var setting = new Guid("e00958c0-c213-4ace-ac77-fecced2eeea5");
        powerRegistration = RegisterPowerSettingNotification(window, ref setting, 0);
        Update("NoriDrive");
        RefreshPower();
    }
    private NotifyData Data(string text) => new()
    {
        Size = (uint)Marshal.SizeOf<NotifyData>(), Window = window, Id = 1, Flags = 1 | 2 | 4,
        CallbackMessage = Callback, Icon = icon, Tip = text[..Math.Min(127, text.Length)], Info = "", InfoTitle = ""
    };
    public void Update(string text)
    {
        var data = Data(text);
        Available = ShellNotifyIcon(Available ? 1u : 0u, ref data);
    }
    public void RefreshPower()
    {
        bool available = GetSystemPowerStatus(out var status);
        power(available && status.SystemStatusFlag == 1, sleeping, available);
    }
    private nint Message(nint hwnd, uint message, nuint wparam, nint lparam, nuint id, nuint data)
    {
        if (message == taskbarCreated) { Available = false; Update("NoriDrive"); }
        if (message == Callback)
        {
            var mouse = (uint)lparam & 0xffff;
            if (mouse is 0x202 or 0x203) action("show");
            if (mouse == 0x205) Menu();
        }
        if (message == 0x218)
        {
            if (wparam == 4) sleeping = true;
            else if (wparam is 7 or 18) sleeping = false;
            RefreshPower();
        }
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }
    private void Menu()
    {
        var menu = CreatePopupMenu();
        var items = new[] { ("show", "tray.show"), ("settings", "tray.settings"), ("toggle", "action.pause"), ("check", "action.check"), ("folder", "action.open"), ("quit", "tray.quit") };
        for (int index = 0; index < items.Length; index++) AppendMenu(menu, 0, (nuint)index + 1, translate(items[index].Item2));
        GetCursorPos(out var cursor); SetForegroundWindow(window);
        var selected = TrackPopupMenuEx(menu, 0x100 | 0x2, cursor.X, cursor.Y, window, 0);
        DestroyMenu(menu);
        if (selected > 0 && selected <= items.Length) action(items[selected - 1].Item1);
    }
    public void Dispose()
    {
        var data = Data(""); ShellNotifyIcon(2, ref data);
        if (powerRegistration != 0) UnregisterPowerSettingNotification(powerRegistration);
        RemoveWindowSubclass(window, callback, 1);
        if (icon != 0) DestroyIcon(icon);
    }
    [DllImport("comctl32.dll")] private static extern bool SetWindowSubclass(nint hwnd, SubclassProc proc, nuint id, nuint data);
    [DllImport("comctl32.dll")] private static extern bool RemoveWindowSubclass(nint hwnd, SubclassProc proc, nuint id);
    [DllImport("comctl32.dll")] private static extern nint DefSubclassProc(nint hwnd, uint msg, nuint wp, nint lp);
    [DllImport("shell32.dll", EntryPoint = "Shell_NotifyIconW")] private static extern bool ShellNotifyIcon(uint message, ref NotifyData data);
    [DllImport("user32.dll", EntryPoint = "LoadImageW", CharSet = CharSet.Unicode)] private static extern nint LoadImage(nint instance, string path, uint type, int cx, int cy, uint flags);
    [DllImport("user32.dll", EntryPoint = "RegisterWindowMessageW", CharSet = CharSet.Unicode)] private static extern uint RegisterWindowMessage(string name);
    [DllImport("user32.dll")] private static extern bool DestroyIcon(nint icon);
    [DllImport("user32.dll")] private static extern nint CreatePopupMenu();
    [DllImport("user32.dll", EntryPoint = "AppendMenuW", CharSet = CharSet.Unicode)] private static extern bool AppendMenu(nint menu, uint flags, nuint id, string text);
    [DllImport("user32.dll")] private static extern int TrackPopupMenuEx(nint menu, uint flags, int x, int y, nint window, nint parameters);
    [DllImport("user32.dll")] private static extern bool DestroyMenu(nint menu);
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(nint window);
    [DllImport("kernel32.dll")] private static extern bool GetSystemPowerStatus(out PowerStatus status);
    [DllImport("user32.dll")] private static extern nint RegisterPowerSettingNotification(nint recipient, ref Guid setting, uint flags);
    [DllImport("user32.dll")] private static extern bool UnregisterPowerSettingNotification(nint handle);
}
