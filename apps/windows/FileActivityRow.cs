using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace FileNode.Windows;

internal sealed class FileActivityRow
{
    public Grid Root { get; } = new() { Padding = new Thickness(16, 12, 16, 12), ColumnSpacing = 12 };
    private readonly TextBlock name = new() { FontSize = 14, TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly TextBlock detail = new() { FontSize = 12, Opacity = .8, TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock time = new() { FontSize = 11, Opacity = .6 };
    private readonly ProgressBar progress = new() { Minimum = 0, Maximum = 100, Height = 3, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Image thumbnail = new() { Width = 40, Height = 40, Stretch = Stretch.Uniform };
    private readonly FontIcon fallback = new() { Glyph = "\uE8A5", FontSize = 34, Opacity = .8 };
    private bool thumbnailRequested;
    public FileActivityRow()
    {
        Root.ColumnDefinitions.Add(new() { Width = new GridLength(40) }); Root.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var icon = new Grid { Width = 40, Height = 48, VerticalAlignment = VerticalAlignment.Top }; icon.Children.Add(fallback); icon.Children.Add(thumbnail); Root.Children.Add(icon);
        var text = new StackPanel { Spacing = 5 }; text.Children.Add(name); text.Children.Add(detail); text.Children.Add(progress); text.Children.Add(time); Grid.SetColumn(text, 1); Root.Children.Add(text);
    }
    public void Update(FileActivity item, CoreClient core)
    {
        name.Text = Path.GetFileName(item.Path);
        var state = item.Active ? item.Operation == "upload" ? "uploading" : item.Operation == "download" ? "downloading" : "working" : item.State;
        detail.Text = core.T("activity." + state) + (item.Active && item.Total > 0 ? $" · {item.Percent:0}%" : "");
        if (item.State == "synced" && item.Operation == "trash") detail.Text = core.T("activity.trashed");
        if (item.State is "retrying" or "warning" or "blocked") detail.Foreground = new SolidColorBrush(Colors.DarkOrange); else detail.ClearValue(TextBlock.ForegroundProperty);
        progress.Visibility = item.Active ? Visibility.Visible : Visibility.Collapsed; progress.IsIndeterminate = item.Active && item.Total == 0; progress.Value = item.Percent;
        time.Text = Path.GetFileName(item.Folder.TrimEnd(Path.DirectorySeparatorChar)) + " · " + item.Updated.ToString("t");
        ToolTipService.SetToolTip(Root, Path.Combine(item.Folder, item.Path.Replace('/', Path.DirectorySeparatorChar)));
        if (!thumbnailRequested)
        {
            var path = Path.Combine(item.Folder, item.Path.Replace('/', Path.DirectorySeparatorChar));
            if (File.Exists(path)) { thumbnailRequested = true; _ = LoadThumbnail(path); }
        }
    }
    private async Task LoadThumbnail(string path)
    {
        try
        {
            var file = await global::Windows.Storage.StorageFile.GetFileFromPathAsync(path);
            using var preview = await file.GetThumbnailAsync(global::Windows.Storage.FileProperties.ThumbnailMode.SingleItem, 48);
            if (preview is null) return;
            var bitmap = new BitmapImage(); await bitmap.SetSourceAsync(preview); thumbnail.Source = bitmap; fallback.Visibility = Visibility.Collapsed;
        }
        catch { /* Locked, removed or remote-only files keep the document icon. */ }
    }
}
