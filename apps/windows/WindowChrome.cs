using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace FileNode.Windows;

internal static class WindowChrome
{
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
