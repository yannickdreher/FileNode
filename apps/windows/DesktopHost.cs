using System.Diagnostics;
using System.Text.Json.Nodes;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics;

namespace FileNode.Windows;

internal sealed class DesktopHost
{
    private readonly Window status = new() { Title = "FileNode" };
    private Window? settings;
    private readonly CoreClient core;
    private readonly NativeTray tray;
    private readonly WindowShutdown statusShutdown;
    private WindowShutdown? settingsShutdown;
    private StackPanel activity = null!;
    private TextBlock summary = null!;
    private TextBlock account = null!;
    private TextBlock error = null!;
    private MenuFlyoutItem toggle = null!;
    private readonly Dictionary<(string, string), FileActivityRow> activityRows = [];
    private readonly List<(string Folder, TextBlock Label)> statusFolders = [], pageFolders = [];
    private readonly List<(string Folder, Expander Details, TextBlock Text)> folderErrors = [];
    private InfoBar? accountStatus;
    private TextBlock? limitsInfo;
    private bool quitting, background, powerPublished;
    private string renderedLanguage = "";
    private NavigationView? navigation;
    private readonly List<Control> editableControls = [];
    private TextBlock? pageError;
    private Button? accountAction;
    private Func<string>? accountActionLabel;
    private Action? refreshFolders;
    public DesktopHost()
    {
        core = new CoreClient(status.DispatcherQueue);
        statusShutdown = new WindowShutdown(status, EndSession);
        status.SystemBackdrop = new MicaBackdrop();
        status.AppWindow.Resize(new SizeInt32(380, 640));

        status.AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "filenode.ico"));
        tray = new NativeTray(WinRT.Interop.WindowNative.GetWindowHandle(status), key => core.T(key == "action.pause" && !core.Running ? "action.start" : key),
            action => Dispatch(() => Act(action)), (saver, sleeping, available) => _ = core.Power(saver, sleeping, available));
        status.AppWindow.Closing += (sender, args) =>
        {
            if (quitting) return;
            args.Cancel = true;
            if (tray.Available) status.AppWindow.Hide(); else _ = Quit();
        };
        if (tray.Available) WindowChrome.ConfigurePopup(status);
        status.Activated += (_, args) =>
        {
            if (args.WindowActivationState != WindowActivationState.Deactivated || !tray.Available) return;
            // Activation can change again while a tray click is being dispatched.
            // Hide only after queued activation has settled, using the current HWND.
            status.DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            {
                if (!quitting && status.AppWindow.IsVisible && !WindowChrome.OwnsForeground(status)) status.AppWindow.Hide();
            });
        };
        core.Changed += Update;
        BuildStatus();
    }
    public void Dispatch(Action action) => status.DispatcherQueue.TryEnqueue(() => action());
    public void Start(string? worker, string? profile, bool startBackground, bool startSettings)
    {
        background = startBackground && !App.ShutdownTest;
        core.Start(worker, profile);
        if (App.ShutdownTest) return;
        if (startSettings) ShowSettings();
        else if (!startBackground || !tray.Available) ShowStatus();
    }
    private TextBlock Label(string key, double size = 14) => new() { Text = core.T(key), FontSize = size, TextWrapping = TextWrapping.Wrap };
    private Button Button(string key, Action action)
    {
        var button = new Button { Content = core.T(key) }; button.Click += (_, _) => action(); return button;
    }
    private void BuildStatus()
    {
        activity = new StackPanel { Spacing = 0 }; activityRows.Clear(); statusFolders.Clear();
        summary = new TextBlock { FontSize = 12 }; account = new TextBlock { FontSize = 11, Opacity = .65, TextTrimming = TextTrimming.CharacterEllipsis };
        error = new TextBlock { FontSize = 12, TextWrapping = TextWrapping.Wrap };
        var root = new Grid();
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var header = new StackPanel { Spacing = 7, Margin = new Thickness(16, 16, 12, 12) };
        var title = new Grid(); title.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); title.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var brand = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        brand.Children.Add(new Image { Source = new BitmapImage(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "icon.png"))), Width = 22, Height = 22 });
        brand.Children.Add(new TextBlock { Text = "FileNode", FontSize = 16, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center }); title.Children.Add(brand);
        var menu = new MenuFlyout();
        void Menu(string key, Action action) { var item = new MenuFlyoutItem { Text = core.T(key) }; item.Click += (_, _) => action(); menu.Items.Add(item); }
        Menu("tray.settings", ShowSettings); menu.Items.Add(new MenuFlyoutSeparator());
        toggle = new MenuFlyoutItem { Text = core.T(core.Running ? "action.pause" : "action.start") }; toggle.Click += Toggle; menu.Items.Add(toggle);
        Menu("action.check", () => _ = core.Send("check")); menu.Items.Add(new MenuFlyoutSeparator()); Menu("tray.quit", () => _ = Quit());
        var gear = new Button { Content = new FontIcon { Glyph = "\uE713", FontSize = 18 }, Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), Padding = new Thickness(8), Flyout = menu };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(gear, core.T("tray.menu")); Grid.SetColumn(gear, 1); title.Children.Add(gear);
        header.Children.Add(title); header.Children.Add(account); header.Children.Add(summary); header.Children.Add(error);
        var folders = new StackPanel { Spacing = 6 };
        foreach (var selection in (core.Config["sync_folders"] as JsonArray ?? []).OfType<JsonObject>()) {
            var path = selection["local_folder"]!.ToString();
            var row = new Grid { ColumnSpacing = 8 }; row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.Children.Add(new TextBlock { Text = Path.GetFileName(path.TrimEnd(Path.DirectorySeparatorChar)), FontSize = 12, TextTrimming = TextTrimming.CharacterEllipsis });
            var state = new TextBlock { FontSize = 11, Opacity = .75 }; Grid.SetColumn(state, 1); row.Children.Add(state); statusFolders.Add((path, state)); folders.Children.Add(row);
        }
        if (statusFolders.Count > 2) header.Children.Add(new Expander { Header = core.T("settings.folders"), Content = folders, HorizontalAlignment = HorizontalAlignment.Stretch }); else header.Children.Add(folders);
        var scroll = new ScrollViewer { Content = activity, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled }; Grid.SetRow(scroll, 1);
        var footer = new Grid { Padding = new Thickness(4, 8, 4, 8), BorderThickness = new Thickness(0, 1, 0, 0), BorderBrush = new SolidColorBrush(Colors.Gray) };
        void Link(int column, string key, Symbol icon, Action action) {
            footer.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var button = new Button { HorizontalAlignment = HorizontalAlignment.Stretch, Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), Padding = new Thickness(2, 10, 2, 10) };
            var content = new StackPanel { Spacing = 7, HorizontalAlignment = HorizontalAlignment.Center }; content.Children.Add(new SymbolIcon(icon) { Width = 20, Height = 20 }); content.Children.Add(new TextBlock { Text = core.T(key), FontSize = 11, HorizontalAlignment = HorizontalAlignment.Center }); button.Content = content;
            button.Click += (_, _) => action(); Grid.SetColumn(button, column); footer.Children.Add(button);
        }
        Link(0, "action.open", Symbol.Folder, OpenSelectedFolders); Link(1, "action.online", Symbol.World, OpenOnline); Link(2, "action.trash", Symbol.Delete, OpenTrash);
        Grid.SetRow(footer, 2); root.Children.Add(header); root.Children.Add(scroll); root.Children.Add(footer);
        var frame = new Border { Child = root, CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1) };
        void Theme() { var dark = frame.ActualTheme == ElementTheme.Dark; frame.Background = new SolidColorBrush(dark ? ColorHelper.FromArgb(255, 32, 32, 32) : ColorHelper.FromArgb(255, 250, 250, 250)); WindowChrome.ThemeBorder(status, dark, popup: tray.Available); frame.BorderBrush = new SolidColorBrush(dark ? ColorHelper.FromArgb(255, 70, 70, 70) : ColorHelper.FromArgb(255, 215, 215, 215)); }
        frame.ActualThemeChanged += (_, _) => Theme(); frame.Loaded += (_, _) => Theme(); status.Content = frame; Theme();
        renderedLanguage = core.Language;
    }
    private void Toggle(object sender, RoutedEventArgs args) => _ = core.Send(core.Running ? "pause" : "connect");
    private void Update(bool configChanged)
    {
        var languageChanged = renderedLanguage != core.Language;
        if (configChanged || languageChanged) { BuildStatus(); if (languageChanged && settings is not null) BuildSettings(); else refreshFolders?.Invoke(); }
        account.Text = core.Authenticated ? core.Text("username") : core.T("account.not_connected");
        summary.Text = core.AccountStatus; error.Text = core.Error == "" ? "" : core.T("error.summary"); error.Visibility = core.Error == "" ? Visibility.Collapsed : Visibility.Visible;
        toggle.Text = core.T(core.Running ? "action.pause" : "action.start"); toggle.IsEnabled = core.Ready;
        foreach (var control in editableControls) control.IsEnabled = core.Ready && core.AccountState != "checking";
        foreach (var (folder, label) in statusFolders.Concat(pageFolders)) { label.Text = core.FolderStatus(folder); ToolTipService.SetToolTip(label, core.FolderStatuses.GetValueOrDefault(folder)?["details"]?.ToString() ?? ""); }
        UpdateFolderErrors();
        if (limitsInfo is not null) limitsInfo.Text = core.LimitsSummary;
        if (accountAction is not null && accountActionLabel is not null) { accountAction.Content = core.T(accountActionLabel()); accountAction.IsEnabled = core.Ready && core.AccountState != "checking"; accountAction.Visibility = Visibility.Visible; }
        if (accountStatus is not null) { accountStatus.Title = core.AccountStatus; accountStatus.Message = core.Authenticated ? core.Text("username") : ""; accountStatus.Severity = core.Authenticated ? InfoBarSeverity.Success : core.AccountState == "failed" ? InfoBarSeverity.Error : InfoBarSeverity.Informational; }
        if (pageError is not null) pageError.Text = core.Error;
        if (core.Activity.Count == 0) {
            if (activityRows.Count > 0) { activityRows.Clear(); activity.Children.Clear(); }
            if (activity.Children.Count == 0) { var empty = Label("activity.empty"); empty.Margin = new Thickness(16, 24, 16, 24); activity.Children.Add(empty); }
        } else {
            var wanted = core.Activity.Select(item => (item.Folder, item.Path)).ToHashSet();
            foreach (var key in activityRows.Keys.Where(key => !wanted.Contains(key)).ToArray()) { activity.Children.Remove(activityRows[key].Root); activityRows.Remove(key); }
            if (activityRows.Count == 0) activity.Children.Clear();
            for (int index = 0; index < core.Activity.Count; index++) {
                var item = core.Activity[index]; var key = (item.Folder, item.Path);
                if (!activityRows.TryGetValue(key, out var row)) { row = new(); activityRows[key] = row; activity.Children.Insert(index, row.Root); }
                row.Update(item, core);
            }
        }
        tray.Update("FileNode — " + core.AccountStatus);
        if (core.Ready && !powerPublished) { powerPublished = true; tray.RefreshPower(); }
        if (background && core.Ready) { background = false; if (core.Text("username") == "") ShowSettings(); }
        if (settings is not null && navigation is null && (configChanged || core.Error != "")) BuildSettings();
    }
    public void ShowStatus()
    {
        if (tray.Available) WindowChrome.ConfigurePopup(status);
        else if (status.AppWindow.Presenter is OverlappedPresenter presenter)
        { presenter.IsMaximizable = false; presenter.IsResizable = false; }
        var area = DisplayArea.GetFromWindowId(status.AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        status.AppWindow.Move(new PointInt32(area.X + area.Width - status.AppWindow.Size.Width - 12, area.Y + area.Height - status.AppWindow.Size.Height - 12));
        WindowChrome.Show(status);
    }
    private void Act(string action)
    {
        switch (action)
        {
            case "show": ShowStatus(); break;
            case "settings": ShowSettings(); break;
            case "toggle": _ = core.Send(core.Running ? "pause" : "connect"); break;
            case "check": _ = core.Send("check"); break;
            case "folder": OpenSelectedFolders(); break;
            case "quit": _ = Quit(); break;
        }
    }
    private static void Open(string path)
    { if (Directory.Exists(path)) Process.Start(new ProcessStartInfo("explorer.exe") { UseShellExecute = true, ArgumentList = { path } }); }
    private void OpenOnline()
    {
        if (Uri.TryCreate(core.Text("web_url"), UriKind.Absolute, out var url) && url.Scheme == "https" && url.UserInfo == "")
            Process.Start(new ProcessStartInfo(url.AbsoluteUri) { UseShellExecute = true });
    }
    private void ShowSettings()
    {
        EnsureSettings();
        WindowChrome.Show(settings!);
    }
    private void EnsureSettings()
    {
        if (settings is null)
        {
            settings = new Window { Title = "FileNode — " + core.T("tray.settings"), SystemBackdrop = new MicaBackdrop() };
            settingsShutdown = new WindowShutdown(settings, EndSession);
            settings.AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "filenode.ico"));
            settings.AppWindow.Resize(new SizeInt32(1000, 800));
            settings.AppWindow.Closing += (_, args) => { args.Cancel = true; settings.AppWindow.Hide(); if (!tray.Available) ShowStatus(); };
            BuildSettings();
        }
    }
    private void BuildSettings()
    {
        if (settings is null) return;
        settings.Title = "FileNode — " + core.T("tray.settings");
        if (!core.Ready)
        {
            navigation = null;
            UIElement loading = core.Error == "" ? new ProgressRing { IsActive = true, Width = 40, Height = 40 } :
                new TextBlock { Text = core.T("error.details") + "\n" + core.Error, Margin = new Thickness(28), TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
            WindowChrome.SetContent(settings, loading, settings.Title);
            return;
        }
        navigation = new NavigationView { IsBackButtonVisible = NavigationViewBackButtonVisible.Collapsed, IsSettingsVisible = false, PaneDisplayMode = NavigationViewPaneDisplayMode.Left, OpenPaneLength = 230 };
        var keys = new[] { "settings.sync", "settings.account", "settings.general", "settings.about" };
        var icons = new[] { Symbol.Sync, Symbol.Contact, Symbol.Setting, Symbol.Help };
        for (int index = 0; index < keys.Length; index++) navigation.MenuItems.Add(new NavigationViewItem { Content = core.T(keys[index]), Tag = index, Icon = new SymbolIcon(icons[index]) });
        navigation.SelectionChanged += (_, args) =>
        {
            if (args.SelectedItem is NavigationViewItem item) ShowPage((int)item.Tag, keys[(int)item.Tag]);
        };
        WindowChrome.SetContent(settings, navigation, settings.Title); navigation.SelectedItem = navigation.MenuItems[0];
    }
    private void ShowPage(int index, string title)
    {
        refreshFolders = null; pageFolders.Clear(); folderErrors.Clear(); accountStatus = null; limitsInfo = null;
        editableControls.Clear(); accountAction = null; accountActionLabel = null;
        var panel = new StackPanel { Spacing = 18, Margin = new Thickness(28), MaxWidth = 760, HorizontalAlignment = HorizontalAlignment.Stretch };
        panel.Children.Add(Label(title, 28));
        if (index == 0) SyncPage(panel);
        if (index == 1) AccountPage(panel);
        if (index == 2) GeneralPage(panel);
        if (index == 3) { panel.Children.Add(Label("about.description")); panel.Children.Add(Label("about.limits")); panel.Children.Add(new TextBlock { Text = "WinUI 3 · Rust · " + typeof(App).Assembly.GetName().Version?.ToString(3) + " Preview" }); }
        pageError = new TextBlock { Text = core.Error, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
        panel.Children.Add(pageError);
        navigation!.Content = new ScrollViewer { Content = panel, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    }
    private TextBox Field(StackPanel panel, string key, string setting)
    {
        var field = new TextBox { Header = core.T(key), Text = core.Text(setting), IsEnabled = core.Ready && core.AccountState != "checking" };
        panel.Children.Add(field); editableControls.Add(field); return field;
    }
    private ToggleSwitch Switch(StackPanel panel, string key, string setting, bool fallback = false)
    {
        var control = new ToggleSwitch { Header = core.T(key), IsOn = core.Flag(setting, fallback) };
        control.Toggled += async (_, _) => await core.Apply(new() { [setting] = control.IsOn });
        panel.Children.Add(control); return control;
    }
    private void SyncPage(StackPanel panel)
    {
        panel.Children.Add(Label("settings.sync_hint"));
        panel.Children.Add(Label("settings.folders", 20));
        panel.Children.Add(Label("settings.folders_hint"));
        var rows = new StackPanel { Spacing = 12 };
        var folderControls = new List<Control>();
        panel.Children.Add(rows);
        refreshFolders = () =>
        {
            foreach (var control in folderControls) editableControls.Remove(control);
            folderControls.Clear();
            rows.Children.Clear(); pageFolders.Clear(); folderErrors.Clear();
            var folders = core.Config["sync_folders"] as JsonArray ?? [];
            if (folders.Count == 0) rows.Children.Add(Label("settings.no_folders"));
            for (int i = 0; i < folders.Count; i++)
            {
                int selected = i;
                var item = folders[i]!;
                var path = item["local_folder"]!.ToString();
                var content = new StackPanel { Spacing = 8 };
                content.Children.Add(new TextBlock { Text = path, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
                content.Children.Add(new TextBlock { Text = core.T("settings.folder_target") + ": /" + item["remote_path"], Opacity = .7, TextWrapping = TextWrapping.Wrap });
                var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                actions.Children.Add(Button("action.open", () => Open(path)));
                var state = new TextBlock { Text = core.FolderStatus(path), FontSize = 13 }; content.Children.Add(state); pageFolders.Add((path, state));
                var detailsText = new TextBlock { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
                var details = new Expander { Header = core.T("error.details"), Content = detailsText, HorizontalAlignment = HorizontalAlignment.Stretch };
                folderErrors.Add((path, details, detailsText)); content.Children.Add(details);
                var remove = Button("settings.remove_folder", () => { var next = (core.Config["sync_folders"] as JsonArray ?? []).DeepClone().AsArray(); next.RemoveAt(selected); _ = core.Apply(new() { ["sync_folders"] = next }); });
                remove.IsEnabled = core.Ready && core.AccountState != "checking";
                editableControls.Add(remove); folderControls.Add(remove); actions.Children.Add(remove); content.Children.Add(actions);
                rows.Children.Add(new Border { Child = content, Padding = new Thickness(16), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Gray) });
            }
            UpdateFolderErrors();
        };
        refreshFolders();
        var add = Button("settings.add_folder", () => _ = AddFolder());
        add.IsEnabled = core.Ready && core.AccountState != "checking"; editableControls.Add(add); panel.Children.Add(add);
        panel.Children.Add(Label("settings.stop_hint"));
        var deletes = new ToggleSwitch { Header = core.T("settings.deletes"), IsOn = core.Flag("propagate_deletes"), IsEnabled = core.Ready && core.AccountState != "checking" }; panel.Children.Add(deletes); editableControls.Add(deletes);
        var patterns = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.NoWrap, Height = 180, Text = string.Join("\n", (core.Config["exclude_patterns"] as JsonArray ?? []).Select(item => item!.ToString())), IsEnabled = core.Ready && core.AccountState != "checking" };
        editableControls.Add(patterns);
        var exclusions = new StackPanel { Spacing = 12 }; exclusions.Children.Add(Label("settings.exclusions_hint")); exclusions.Children.Add(patterns);
        panel.Children.Add(new Expander { Header = core.T("settings.exclusions"), Content = exclusions, HorizontalAlignment = HorizontalAlignment.Stretch });
        var maxFile = new NumberBox { Header = core.T("settings.max_file_size"), Minimum = 0, Maximum = 16777216, Value = Math.Ceiling((double)CoreClient.Unsigned(core.Config["max_file_bytes"]) / 1048576), IsEnabled = core.Ready && core.AccountState != "checking", SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        var parallel = new NumberBox { Header = core.T("settings.parallel_uploads"), Minimum = 1, Maximum = 16, Value = core.Number("parallel_uploads", 2), IsEnabled = core.Ready && core.AccountState != "checking", SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        panel.Children.Add(maxFile); panel.Children.Add(parallel); editableControls.Add(maxFile); editableControls.Add(parallel);
        limitsInfo = new TextBlock { Text = core.LimitsSummary, TextWrapping = TextWrapping.Wrap }; panel.Children.Add(limitsInfo);
        panel.Children.Add(Label("settings.bandwidth", 20)); Rate(panel, "settings.download_limit", "download_limit_kib"); Rate(panel, "settings.upload_limit", "upload_limit_kib");
        Switch(panel, "settings.pause_power", "pause_on_power_saver", true);
        var save = Button("settings.save", () => _ = core.Apply(new() { ["propagate_deletes"] = deletes.IsOn, ["exclude_patterns"] = new JsonArray(patterns.Text.Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).Select(item => (JsonNode?)JsonValue.Create(item)).ToArray()), ["max_file_bytes"] = double.IsFinite(maxFile.Value) ? (ulong)Math.Round(maxFile.Value) * 1048576 : 0, ["parallel_uploads"] = double.IsFinite(parallel.Value) ? (int)parallel.Value : 2 }));
        save.IsEnabled = core.Ready && core.AccountState != "checking"; panel.Children.Add(save); editableControls.Add(save);
        if (core.Running) panel.Children.Add(Label("settings.pause_hint"));
    }
    private void UpdateFolderErrors()
    {
        foreach (var (folder, details, text) in folderErrors) {
            text.Text = core.FolderStatuses.GetValueOrDefault(folder)?["details"]?.ToString() ?? "";
            details.Visibility = text.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        }
    }
    private async Task AddFolder()
    {
        if (settings is null) return;
        try
        {
            var picker = new global::Windows.Storage.Pickers.FolderPicker();
            picker.FileTypeFilter.Add("*");
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(settings));
            var folder = await picker.PickSingleFolderAsync();
            if (folder is null) return;
            var next = (core.Config["sync_folders"] as JsonArray ?? []).DeepClone().AsArray();
            next.Add(new JsonObject { ["local_folder"] = folder.Path, ["remote_path"] = "" });
            await core.Apply(new() { ["sync_folders"] = next }, connect: core.Authenticated);
        }
        catch (Exception exception) { if (pageError is not null) pageError.Text = exception.Message; }
    }
    private void OpenSelectedFolders()
    {
        var folders = core.Config["sync_folders"] as JsonArray ?? [];
        if (folders.Count == 1) Open(folders[0]!["local_folder"]!.ToString());
        else { ShowSettings(); if (navigation is not null) navigation.SelectedItem = navigation.MenuItems[0]; }
    }
    private void OpenTrash()
    {
        var address = string.IsNullOrEmpty(core.TrashUrl) ? core.Text("web_url") : core.TrashUrl;
        if (Uri.TryCreate(address, UriKind.Absolute, out var url) && url.Scheme == "https" && url.UserInfo == "") Process.Start(new ProcessStartInfo(url.AbsoluteUri) { UseShellExecute = true });
    }
    private void Rate(StackPanel panel, string label, string key)
    {
        var row = new Grid { ColumnSpacing = 18 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(180) });
        var enabled = new ToggleSwitch { Header = core.T(label), IsOn = core.Number(key) > 0 };
        var rate = new NumberBox { Header = "KiB/s", Minimum = 1, Maximum = 1_000_000, Value = core.Number(key) > 0 ? core.Number(key) : 1024, IsEnabled = enabled.IsOn, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        enabled.Toggled += async (_, _) => { rate.IsEnabled = enabled.IsOn; await core.Apply(new() { [key] = enabled.IsOn ? (int)rate.Value : 0 }); };
        rate.ValueChanged += async (_, args) => { if (enabled.IsOn && double.IsFinite(args.NewValue)) await core.Apply(new() { [key] = (int)args.NewValue }); };
        Grid.SetColumn(rate, 1); row.Children.Add(enabled); row.Children.Add(rate); panel.Children.Add(row);
    }
    private void AccountPage(StackPanel panel)
    {
        accountStatus = new InfoBar { IsOpen = true, IsClosable = false, Title = core.AccountStatus, Message = core.Authenticated ? core.Text("username") : "", Severity = core.Authenticated ? InfoBarSeverity.Success : InfoBarSeverity.Informational }; panel.Children.Add(accountStatus);
        var server = Field(panel, "settings.server", "server"); var web = Field(panel, "settings.web", "web_url"); var username = Field(panel, "settings.username", "username");
        var mode = new ComboBox { Header = core.T("settings.authentication"), ItemsSource = new[] { core.T("settings.auth_password"), core.T("settings.auth_browser") }, SelectedIndex = core.Text("auth_mode") == "browser" ? 1 : 0, IsEnabled = core.Ready && core.AccountState != "checking" };
        panel.Children.Add(mode); editableControls.Add(mode);
        var credentials = new StackPanel { Spacing = 12 };
        var password = new PasswordBox { Header = core.T("settings.password"), PlaceholderText = core.T("settings.password_hint"), IsEnabled = core.Ready && core.AccountState != "checking" }; credentials.Children.Add(password); editableControls.Add(password); credentials.Children.Add(Label("settings.credentials")); panel.Children.Add(credentials);
        var provider = new StackPanel { Spacing = 12 };
        provider.Children.Add(Label("settings.browser_hint"));
        var issuer = Field(provider, "settings.oauth_issuer", "oauth_issuer");
        issuer.PlaceholderText = "https://auth.example.org/application/o/mail/";
        var clientId = Field(provider, "settings.oauth_client", "oauth_client_id");
        var advanced = new Expander { Header = core.T("settings.oauth_advanced"), HorizontalAlignment = HorizontalAlignment.Stretch };
        var oauth = new StackPanel { Spacing = 12 };
        var scopes = Field(oauth, "settings.oauth_scopes", "oauth_scopes");
        var port = new NumberBox { Header = core.T("settings.oauth_port"), Minimum = 0, Maximum = 65535, Value = core.Number("oauth_redirect_port"), IsEnabled = core.Ready && core.AccountState != "checking" };
        oauth.Children.Add(port); editableControls.Add(port);
        advanced.Content = oauth; provider.Children.Add(advanced); panel.Children.Add(provider);
        JsonObject Values() => new() { ["server"] = server.Text, ["web_url"] = web.Text, ["username"] = username.Text,
            ["auth_mode"] = mode.SelectedIndex == 1 ? "browser" : "app_password", ["oauth_issuer"] = issuer.Text,
            ["oauth_client_id"] = clientId.Text, ["oauth_scopes"] = scopes.Text, ["oauth_redirect_port"] = double.IsFinite(port.Value) ? (int)port.Value : 0 };
        accountActionLabel = () => core.AccountState == "checking" ? "account.checking" : core.Authenticated ? "account.signin_again" : "action.signin";
        accountAction = Button(accountActionLabel(), () =>
        {
            _ = core.Apply(Values(), true, mode.SelectedIndex == 1 ? null : password.Password, mode.SelectedIndex == 1); password.Password = "";
        });
        accountAction.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        accountAction.IsEnabled = core.Ready && core.AccountState != "checking";
        void UpdateMode()
        {
            provider.Visibility = mode.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
            credentials.Visibility = mode.SelectedIndex == 1 ? Visibility.Collapsed : Visibility.Visible;
            accountAction.Content = core.T(accountActionLabel());
        }
        mode.SelectionChanged += (_, _) => UpdateMode(); UpdateMode();
        accountAction.Visibility = Visibility.Visible;
        panel.Children.Add(accountAction);
    }
    private void GeneralPage(StackPanel panel)
    {
        var language = new ComboBox { Header = core.T("language.label"), ItemsSource = new[] { core.T("language.auto"), "English", "Deutsch" }, SelectedIndex = core.Text("language", "auto") switch { "en" => 1, "de" => 2, _ => 0 } };
        language.SelectionChanged += async (_, _) =>
        { if (language.SelectedIndex is >= 0 and < 3) await core.Apply(new() { ["language"] = new[] { "auto", "en", "de" }[language.SelectedIndex] }); }; panel.Children.Add(language);
        var startup = new ToggleSwitch { Header = core.T("settings.autostart"), IsOn = core.Flag("autostart") };
        startup.Toggled += async (_, _) =>
        {
            try
            {
                bool enabled = startup.IsOn;
                if (App.IsPackaged)
                {
                    var task = await global::Windows.ApplicationModel.StartupTask.GetAsync("FileNodeStartup");
                    if (enabled) { var state = await task.RequestEnableAsync(); enabled = state == global::Windows.ApplicationModel.StartupTaskState.Enabled; }
                    else task.Disable();
                }
                await core.Apply(new() { ["autostart"] = enabled });
            }
            catch (Exception exception) { panel.Children.Add(new TextBlock { Text = exception.Message, TextWrapping = TextWrapping.Wrap }); }
        };
        panel.Children.Add(startup); panel.Children.Add(Label("settings.background_hint"));
    }
    private async Task Quit()
    {
        if (quitting) return;
        quitting = true; tray.Dispose(); await core.DisposeAsync(); settings?.Close(); status.Close(); Application.Current.Exit();
    }
    private void EndSession()
    {
        if (!quitting) { quitting = true; tray.Dispose(); }
        // Windows may terminate this process when WM_ENDSESSION returns. Drain
        // the worker first; its I/O continuations must not require this UI thread.
        try { core.StopAsync(TimeSpan.FromSeconds(4)).GetAwaiter().GetResult(); }
        catch (Exception error) { App.RecordSelfTestFailure(error); }
        Application.Current.Exit();
    }
    internal async Task PrepareShutdownTest(string directory)
    {
        var limit = DateTime.UtcNow.AddSeconds(15);
        while (!core.Ready && DateTime.UtcNow < limit) await Task.Delay(25);
        if (!core.Ready || core.WorkerId is null || core.Authenticated || core.Running)
            throw new InvalidOperationException("Isolated shutdown worker was not ready");
        EnsureSettings();
        status.AppWindow.Show(false);
        settings!.AppWindow.Show(false);
        File.WriteAllText(Path.Combine(directory, "shutdown-ready.json"), System.Text.Json.JsonSerializer.Serialize(new
        {
            ui = Environment.ProcessId, worker = core.WorkerId,
            status = WinRT.Interop.WindowNative.GetWindowHandle(status).ToInt64(),
            settings = WinRT.Interop.WindowNative.GetWindowHandle(settings).ToInt64(), tray = tray.Available
        }));
    }
    internal async Task RunSmokeTest(string directory)
    {
        App.SelfTestProgress("wait for worker");
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!core.Ready && DateTime.UtcNow < deadline) await Task.Delay(50);
        if (!core.Ready || core.Error != "") throw new InvalidOperationException("UI smoke test worker did not become ready: " + core.Error);
        App.SelfTestProgress("open status and settings");
        ShowStatus(); ShowSettings();
        await Task.Delay(100);
        // A desktop test launched in the background cannot override Windows'
        // foreground lock. Report that limitation rather than failing shutdown/UI checks.
        var testTrayActivation = tray.Available && WindowChrome.OwnsForeground(settings!);
        if (tray.Available)
        {
            App.SelfTestProgress("native popup frame and repeated tray activation");
            async Task WaitForPopup(Func<bool> ready, string message)
            {
                var limit = DateTime.UtcNow.AddSeconds(2);
                while (!ready() && DateTime.UtcNow < limit) await Task.Delay(25);
                if (!ready()) throw new InvalidOperationException(message + $" (visible={status.AppWindow.IsVisible}, statusForeground={WindowChrome.OwnsForeground(status)}, settingsForeground={WindowChrome.OwnsForeground(settings!)})");
            }
            var content = (FrameworkElement)status.Content;
            var previousTheme = content.RequestedTheme;
            foreach (var theme in new[] { ElementTheme.Dark, ElementTheme.Light })
            {
                content.RequestedTheme = theme;
                if (!testTrayActivation)
                {
                    ShowStatus(); await Task.Delay(100); WindowChrome.VerifyPopupFrame(status);
                    continue;
                }
                for (var repeat = 0; repeat < 3; repeat++)
                {
                    ShowSettings();
                    await WaitForPopup(() => WindowChrome.OwnsForeground(settings!) && !status.AppWindow.IsVisible, "Tray popup did not dismiss after losing focus");
                    Dispatch(() => Act("show"));
                    await WaitForPopup(() => status.AppWindow.IsVisible && WindowChrome.OwnsForeground(status), "Tray popup did not reopen in the foreground");
                    WindowChrome.VerifyPopupFrame(status);
                    // A pending deactivation must not hide the newly reopened panel.
                    ShowSettings(); Act("show"); await Task.Delay(100);
                    await WaitForPopup(() => status.AppWindow.IsVisible && WindowChrome.OwnsForeground(status), "Stale deactivation hid the reopened tray popup");
                    WindowChrome.VerifyPopupFrame(status);
                }
            }
            content.RequestedTheme = previousTheme;
        }
        async Task WaitForFolders(int count)
        {
            var limit = DateTime.UtcNow.AddSeconds(5);
            while ((core.Config["sync_folders"]?.AsArray().Count != count || core.Error != "") && DateTime.UtcNow < limit) await Task.Delay(50);
            if (core.Config["sync_folders"]?.AsArray().Count != count || core.Error != "") throw new InvalidOperationException("Folder configuration failed: " + core.Error);
        }
        var first = Path.GetFullPath(Path.Combine(directory, "folders", "Desktop"));
        var second = Path.GetFullPath(Path.Combine(directory, "folders", "Documents", "Büro"));
        Directory.CreateDirectory(first); Directory.CreateDirectory(second);
        File.WriteAllText(Path.Combine(first, "keep.txt"), "UI smoke fixture");
        await core.Apply(new() { ["sync_folders"] = new JsonArray(
            new JsonObject { ["local_folder"] = first, ["remote_path"] = "" },
            new JsonObject { ["local_folder"] = second, ["remote_path"] = "" }) });
        await WaitForFolders(2);
        if (core.Config["sync_folders"]!.AsArray().Any(folder => string.IsNullOrEmpty(folder!["remote_path"]!.ToString()))) throw new InvalidOperationException("Server folder paths were not resolved");
        var keys = new[] { "settings.sync", "settings.account", "settings.general", "settings.about" };
        foreach (var language in new[] { "en", "de" })
        {
            await core.Apply(new() { ["language"] = language });
            while (core.Language != language && DateTime.UtcNow < deadline) await Task.Delay(50);
            if (core.Language != language) throw new InvalidOperationException("UI smoke test language switch timed out");
            foreach (var key in keys.Select((key, index) => (key, index)))
            {
                App.SelfTestProgress(language + ": " + key.key);
                navigation!.SelectedItem = navigation.MenuItems[key.index];
                await Task.Delay(150);
                (settings!.Content as FrameworkElement)?.UpdateLayout();
            }
            App.SelfTestProgress(language + ": account, folder and file states");
            core.Config["username"] = "user@example.org";
            core.Receive(new() { ["event"] = "account", ["state"] = "checking" });
            if (core.Authenticated) throw new InvalidOperationException("An unchecked account was shown as signed in");
            core.Receive(new() { ["event"] = "account", ["state"] = "signed_in" });
            core.Receive(new() { ["event"] = "running", ["value"] = true });
            core.Receive(new() { ["event"] = "folder", ["folder"] = second, ["state"] = "retrying", ["retry_seconds"] = 5, ["details"] = "Offline test fixture" });
            void FileEvent(string state, long bytes) => core.Receive(new() { ["event"] = "file", ["folder"] = first, ["path"] = "Project notes.md", ["operation"] = "upload", ["state"] = state, ["completed"] = bytes, ["total"] = 1024L });
            FileEvent("transferring", 0); FileEvent("transferring", 512);
            if (core.Activity.Count != 1 || core.Activity[0].Percent != 50) throw new InvalidOperationException("Progress created duplicate file activities");
            ShowStatus(); await Task.Delay(150); await Capture(status, directory, language + "-transfer.png");
            FileEvent("synced", 1024);
            FileEvent("waiting", 0);
            core.Receive(new() { ["event"] = "folder", ["folder"] = first, ["state"] = "current" });
            if (core.Activity.Count != 1 || core.Activity[0].State != "synced") throw new InvalidOperationException("Completion did not update the existing file activity");
            core.Receive(new() { ["event"] = "renamed", ["folder"] = first, ["from"] = "Project notes.md", ["to"] = "Project plan.md" });
            if (core.Activity.Count != 1 || core.Activity[0].Path != "Project plan.md") throw new InvalidOperationException("Renaming duplicated the file activity");
            core.Receive(new() { ["event"] = "folder", ["folder"] = first, ["state"] = "current" });
            core.Receive(new() { ["event"] = "file", ["folder"] = second, ["path"] = "Report.pdf", ["operation"] = "download", ["state"] = "retrying", ["completed"] = 1024L, ["total"] = 4096L });
            await Capture(status, directory, language + "-status.png");
            navigation!.SelectedItem = navigation.MenuItems[1]; await Task.Delay(150);
            if (!core.Authenticated || accountStatus?.Title != core.T("account.signed_in") || accountAction?.Content?.ToString() == core.T("action.pause")) throw new InvalidOperationException("Account status is mixed with synchronization controls");
            await Capture(settings!, directory, language + "-account.png");
            navigation.SelectedItem = navigation.MenuItems[0]; await Task.Delay(150);
            if (editableControls.Any(control => !control.IsEnabled)) throw new InvalidOperationException("Sync settings were disabled while synchronization was running");
            if (pageFolders.Count != 2 || pageFolders[0].Label.Text == pageFolders[1].Label.Text) throw new InvalidOperationException("Folder states are not independent");
            var folderError = folderErrors.Single(item => item.Folder == second);
            if (folderError.Details.Visibility != Visibility.Visible || folderError.Text.Text != "Offline test fixture") throw new InvalidOperationException("Folder error details are not accessible");
            folderError.Details.IsExpanded = true;
            await Task.Delay(250); // Let the native Expander's reveal animation complete.
            await Capture(settings!, directory, language + "-folders.png");
            core.Receive(new() { ["event"] = "folder", ["folder"] = second, ["state"] = "current" });
            if (folderError.Details.Visibility != Visibility.Collapsed) throw new InvalidOperationException("Resolved folder error details stayed visible");
            core.Receive(new() { ["event"] = "limits", ["server"] = new JsonObject { ["max_size_upload"] = 10485760L, ["max_concurrent_upload"] = 4 }, ["effective_max_file_bytes"] = 10485760L, ["effective_parallel_uploads"] = 2 });
            for (int repeat = 0; repeat < 2; repeat++) core.Receive(new() { ["event"] = "file_limit", ["folder"] = first, ["path"] = "Large archive.zip", ["size"] = 20971520L, ["limit"] = 10485760L });
            if (core.Activity.Count(item => item.Path == "Large archive.zip") != 1 || core.Activity.Single(item => item.Path == "Large archive.zip").State != "too_large") throw new InvalidOperationException("File-size warnings are missing or duplicated");
            core.Receive(new() { ["event"] = "folder", ["folder"] = first, ["state"] = "limited" });
            await Capture(status, directory, language + "-size-limit.png");
            core.Receive(new() { ["event"] = "running", ["value"] = false });
            if (!core.Authenticated) throw new InvalidOperationException("Pausing sync signed out the account");
            core.Receive(new() { ["event"] = "account", ["state"] = "signed_out" });
            core.Config["username"] = "";
        }
        App.SelfTestProgress("stop one folder without deleting files");
        var remaining = core.Config["sync_folders"]!.DeepClone().AsArray(); remaining.RemoveAt(0);
        await core.Apply(new() { ["sync_folders"] = remaining });
        await WaitForFolders(1);
        if (!File.Exists(Path.Combine(first, "keep.txt")) || !Directory.Exists(second)) throw new InvalidOperationException("Stopping sync removed a selected folder");
        if (core.Running) throw new InvalidOperationException("Offline UI smoke test unexpectedly started synchronization");
        File.WriteAllText(Path.Combine(directory, "ui-smoke.json"), System.Text.Json.JsonSerializer.Serialize(new
        { passed = true, pages = 4, languages = 2, folders = 2, stopPreservesFiles = true, independentStates = true, uniqueFileRows = true, nativePopupFrame = tray.Available, repeatedTrayActivation = testTrayActivation }));
        App.SelfTestProgress("quit");
        await Quit();
    }
    private static async Task Capture(Window window, string directory, string name)
    {
        var content = (FrameworkElement)window.Content; content.UpdateLayout();
        var image = new RenderTargetBitmap(); await image.RenderAsync(content);
        var buffer = await image.GetPixelsAsync(); var bytes = new byte[buffer.Length];
        using (var reader = global::Windows.Storage.Streams.DataReader.FromBuffer(buffer)) reader.ReadBytes(bytes);
        var folder = await global::Windows.Storage.StorageFolder.GetFolderFromPathAsync(Path.GetFullPath(directory));
        var file = await folder.CreateFileAsync(name, global::Windows.Storage.CreationCollisionOption.ReplaceExisting);
        using var stream = await file.OpenAsync(global::Windows.Storage.FileAccessMode.ReadWrite);
        var encoder = await global::Windows.Graphics.Imaging.BitmapEncoder.CreateAsync(global::Windows.Graphics.Imaging.BitmapEncoder.PngEncoderId, stream);
        encoder.SetPixelData(global::Windows.Graphics.Imaging.BitmapPixelFormat.Bgra8, global::Windows.Graphics.Imaging.BitmapAlphaMode.Premultiplied, (uint)image.PixelWidth, (uint)image.PixelHeight, 96, 96, bytes);
        await encoder.FlushAsync();
    }
}
