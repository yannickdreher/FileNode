using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Runtime.InteropServices;

namespace FileNode.Windows;

internal static class WindowChrome
{
    private const int WindowStyle = -16;
    private const long DialogFrame = 0x00400000;

    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(nint window, uint attribute, ref uint value, uint size);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    private static extern nint GetWindowLongPtr(nint window, int index);
    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW", SetLastError = true)]
    private static extern nint SetWindowLongPtr(nint window, int index, nint value);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool SetWindowPos(nint window, nint after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")]
    private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")]
    private static extern nint GetAncestor(nint window, uint flags);
    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(nint window);

    public static void ConfigurePopup(Window window)
    {
        if (window.AppWindow.Presenter is not OverlappedPresenter presenter) return;
        presenter.IsResizable = false;
        presenter.IsMaximizable = false;
        presenter.IsMinimizable = false;
        presenter.SetBorderAndTitleBar(false, false);
        window.AppWindow.IsShownInSwitchers = false;
        RemoveDialogFrame(WinRT.Interop.WindowNative.GetWindowHandle(window));
    }

    private static void RemoveDialogFrame(nint handle)
    {
        // The WinUI non-resizable presenter can leave WS_DLGFRAME behind even
        // with HasBorder=false. This classic frame is separate from the DWM border.
        var style = GetWindowLongPtr(handle, WindowStyle).ToInt64();
        if ((style & DialogFrame) == 0) return;
        Marshal.SetLastPInvokeError(0);
        if (SetWindowLongPtr(handle, WindowStyle, (nint)(style & ~DialogFrame)) == 0 && Marshal.GetLastPInvokeError() != 0)
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastPInvokeError());
        // Recalculate the non-client area without moving, resizing or activating.
        if (!SetWindowPos(handle, 0, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0004 | 0x0010 | 0x0020))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastPInvokeError());
    }

    public static bool OwnsForeground(Window window)
    {
        var handle = WinRT.Interop.WindowNative.GetWindowHandle(window);
        var foreground = GetForegroundWindow();
        return foreground == handle || (foreground != 0 && GetAncestor(foreground, 3) == handle);
    }

    public static void Show(Window window)
    {
        if (window.AppWindow.Presenter is OverlappedPresenter presenter && presenter.State == OverlappedPresenterState.Minimized)
            presenter.Restore();
        // Activate only after revealing a previously hidden HWND.
        window.AppWindow.Show(true);
        window.Activate();
        _ = SetForegroundWindow(WinRT.Interop.WindowNative.GetWindowHandle(window));
    }

    internal static void VerifyPopupFrame(Window window)
    {
        var style = GetWindowLongPtr(WinRT.Interop.WindowNative.GetWindowHandle(window), WindowStyle).ToInt64();
        if ((style & (DialogFrame | 0x00800000 | 0x00040000)) != 0)
            throw new InvalidOperationException($"Tray popup retained a native frame: 0x{style:X8}");
        if (window.AppWindow.IsShownInSwitchers)
            throw new InvalidOperationException("Tray popup appeared in the taskbar/window switcher");
    }

    public static void ThemeBorder(Window window, bool dark, bool popup = false)
    {
        var handle = WinRT.Interop.WindowNative.GetWindowHandle(window);
        if (popup) RemoveDialogFrame(handle);
        uint immersiveDark = dark ? 1u : 0u;
        _ = DwmSetWindowAttribute(handle, 20, ref immersiveDark, sizeof(uint));
        // A tray panel draws its own rounded outline; suppress the extra DWM border.
        uint border = popup ? 0xFFFFFFFEu : dark ? 0x00464646u : 0x00D7D7D7u;
        _ = DwmSetWindowAttribute(handle, 34, ref border, sizeof(uint));
    }
    // Keep the OS caption buttons and Snap behavior, with the XAML theme behind them.
    public static void SetContent(Window window, UIElement content, string title)
    {
        window.ExtendsContentIntoTitleBar = true;
        var root = new Grid(); root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var bar = new Grid { Height = 40, Padding = new Thickness(16, 0, 150, 0), Background = new SolidColorBrush(Colors.Transparent) };
        var brand = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        brand.Children.Add(new Image { Source = new BitmapImage(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "icon.png"))), Width = 16, Height = 16 });
        brand.Children.Add(new TextBlock { Text = title, FontSize = 12, VerticalAlignment = VerticalAlignment.Center }); bar.Children.Add(brand);
        root.Children.Add(bar); Grid.SetRow(content as FrameworkElement ?? throw new ArgumentException("Window content must be a framework element"), 1); root.Children.Add(content);
        window.Content = root; window.SetTitleBar(bar);
        void Theme()
        {
            var dark = root.ActualTheme == ElementTheme.Dark; var caption = window.AppWindow.TitleBar;
            ThemeBorder(window, dark);
            root.Background = new SolidColorBrush(dark ? ColorHelper.FromArgb(255, 32, 32, 32) : ColorHelper.FromArgb(255, 243, 243, 243));
            caption.ButtonBackgroundColor = Colors.Transparent; caption.ButtonInactiveBackgroundColor = Colors.Transparent;
            caption.ButtonForegroundColor = dark ? Colors.White : Colors.Black;
            caption.ButtonInactiveForegroundColor = dark ? Colors.Gray : Colors.DimGray;
            caption.ButtonHoverBackgroundColor = dark ? ColorHelper.FromArgb(30, 255, 255, 255) : ColorHelper.FromArgb(20, 0, 0, 0);
            caption.ButtonHoverForegroundColor = dark ? Colors.White : Colors.Black;
            caption.ButtonPressedBackgroundColor = dark ? ColorHelper.FromArgb(45, 255, 255, 255) : ColorHelper.FromArgb(35, 0, 0, 0);
            caption.ButtonPressedForegroundColor = dark ? Colors.White : Colors.Black;
        }
        root.ActualThemeChanged += (_, _) => Theme(); root.Loaded += (_, _) => Theme(); Theme();
    }
}
