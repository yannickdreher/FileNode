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
    private StackPanel activity = null!;
    private TextBlock summary = null!;
    private TextBlock account = null!;
    private TextBlock error = null!;
    private Button toggle = null!;
    private bool quitting, background, powerPublished;
    private string renderedLanguage = "";
    private NavigationView? navigation;
    private readonly List<Control> pausedControls = [];
    private TextBlock? pageError;
    private Button? accountAction;
    private Func<string>? accountActionLabel;
    private Action? refreshFolders;
    public DesktopHost()
    {
        core = new CoreClient(status.DispatcherQueue);
        status.SystemBackdrop = new MicaBackdrop();
        status.AppWindow.Resize(new SizeInt32(440, 650));
        status.AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "filenode.ico"));
        tray = new NativeTray(WinRT.Interop.WindowNative.GetWindowHandle(status), key => core.T(key == "action.pause" && !core.Running ? "action.resume" : key),
            action => Dispatch(() => Act(action)), (saver, sleeping, available) => _ = core.Power(saver, sleeping, available));
        status.AppWindow.Closing += (sender, args) =>
        {
            if (quitting) return;
            args.Cancel = true;
            if (tray.Available) status.AppWindow.Hide(); else _ = Quit();
        };
        core.Changed += Update;
        BuildStatus();
    }
    public void Dispatch(Action action) => status.DispatcherQueue.TryEnqueue(() => action());
    public void Start(string? worker, string? profile, bool startBackground, bool startSettings)
    {
        background = startBackground;
        core.Start(worker, profile);
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
        // A language change rebuilds this tree. WinUI elements cannot be added
        // to a new parent while they still belong to the previous tree.
        activity = new StackPanel { Spacing = 12 };
        summary = new TextBlock { TextWrapping = TextWrapping.Wrap };
        account = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis };
        error = new TextBlock { TextWrapping = TextWrapping.Wrap };
        toggle = new Button();
        var root = new Grid { Padding = new Thickness(20), RowSpacing = 14 };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var header = new StackPanel { Spacing = 10 };
        var title = new Grid(); title.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); title.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var brand = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        brand.Children.Add(new Image { Source = new BitmapImage(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "icon.png"))), Width = 28, Height = 28 });
        brand.Children.Add(new TextBlock { Text = "FileNode", FontSize = 22 });
        title.Children.Add(brand);
        var gear = new Button { Content = new SymbolIcon(Symbol.Setting) }; gear.Click += (_, _) => ShowSettings(); Grid.SetColumn(gear, 1); title.Children.Add(gear);
        header.Children.Add(title); header.Children.Add(account); header.Children.Add(summary); header.Children.Add(error);
        var scroll = new ScrollViewer { Content = activity, VerticalScrollBarVisibility = ScrollBarVisibility.Auto }; Grid.SetRow(scroll, 1);
        var footer = new StackPanel { Spacing = 8 };
        var links = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        links.Children.Add(Button("action.open", OpenSelectedFolders));
        links.Children.Add(Button("action.online", OpenOnline)); links.Children.Add(Button("action.recovery", OpenSelectedRecovery));
        footer.Children.Add(links);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        toggle.Click += Toggle; actions.Children.Add(toggle);
        actions.Children.Add(Button("action.check", () => _ = core.Send("check")));
        actions.Children.Add(Button("tray.quit", () => _ = Quit())); footer.Children.Add(actions); Grid.SetRow(footer, 2);
        root.Children.Add(header); root.Children.Add(scroll); root.Children.Add(footer); status.Content = root;
        renderedLanguage = core.Language;
    }
    private void Toggle(object sender, RoutedEventArgs args) => _ = core.Send(core.Running ? "pause" : "connect");
    private void Update(bool configChanged)
    {
        if (configChanged) refreshFolders?.Invoke();
        if (renderedLanguage != core.Language) { toggle.Click -= Toggle; BuildStatus(); if (settings is not null) BuildSettings(); }
        account.Text = core.Text("username", core.T("account.not_connected"));
        summary.Text = core.T(core.StatusKey); error.Text = core.Error == "" ? "" : core.T("error.summary");
        toggle.Content = core.T(core.Running ? "action.pause" : "action.resume"); toggle.IsEnabled = core.Ready;
        foreach (var control in pausedControls) control.IsEnabled = !core.Running;
        if (accountAction is not null && accountActionLabel is not null) accountAction.Content = core.T(accountActionLabel());
        if (pageError is not null) pageError.Text = core.Error;
        activity.Children.Clear();
        if (core.Activity.Count == 0)
        {
            activity.Children.Add(Label("status.welcome", 24)); activity.Children.Add(Label("status.setup_hint"));
            activity.Children.Add(Button("tray.settings", ShowSettings)); activity.Children.Add(Label("activity.empty"));
        }
        foreach (var item in core.Activity)
        {
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(new TextBlock { Text = core.Format(item["message"]!.AsObject()), TextWrapping = TextWrapping.Wrap });
            row.Children.Add(new TextBlock { Text = item["time"]!.ToString(), Opacity = .65, FontSize = 12 }); activity.Children.Add(row);
        }
        tray.Update("FileNode — " + core.T(core.StatusKey));
        if (core.Ready && !powerPublished) { powerPublished = true; tray.RefreshPower(); }
        if (background && core.Ready)
        {
            background = false;
            if (core.Text("username") == "") ShowSettings();
        }
        if (settings is not null && navigation is null && (configChanged || core.Error != "")) BuildSettings();
    }
    public void ShowStatus()
    {
        if (status.AppWindow.Presenter is OverlappedPresenter presenter)
        { presenter.IsMaximizable = false; presenter.IsResizable = false; }
        var area = DisplayArea.GetFromWindowId(status.AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        status.AppWindow.Move(new PointInt32(area.X + area.Width - status.AppWindow.Size.Width - 12, area.Y + area.Height - status.AppWindow.Size.Height - 12));
        status.Activate(); status.AppWindow.Show();
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
        if (settings is null)
        {
            settings = new Window { Title = "FileNode — " + core.T("tray.settings"), SystemBackdrop = new MicaBackdrop() };
            settings.AppWindow.Resize(new SizeInt32(1000, 800));
            settings.AppWindow.Closing += (_, args) => { args.Cancel = true; settings.AppWindow.Hide(); if (!tray.Available) ShowStatus(); };
            BuildSettings();
        }
        settings.Activate(); settings.AppWindow.Show();
    }
    private void BuildSettings()
    {
        if (settings is null) return;
        if (!core.Ready)
        {
            navigation = null;
            settings.Content = core.Error == "" ? new ProgressRing { IsActive = true, Width = 40, Height = 40 } :
                new TextBlock { Text = core.T("error.details") + "\n" + core.Error, Margin = new Thickness(28), TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
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
        settings.Content = navigation; navigation.SelectedItem = navigation.MenuItems[0];
    }
    private void ShowPage(int index, string title)
    {
        refreshFolders = null;
        pausedControls.Clear(); accountAction = null; accountActionLabel = null;
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
        var field = new TextBox { Header = core.T(key), Text = core.Text(setting), IsEnabled = !core.Running };
        panel.Children.Add(field); pausedControls.Add(field); return field;
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
            foreach (var control in folderControls) pausedControls.Remove(control);
            folderControls.Clear();
            rows.Children.Clear();
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
                actions.Children.Add(Button("action.recovery", () => Open(Path.Combine(path, ".filenode", "history"))));
                var remove = Button("settings.remove_folder", () => { var next = (core.Config["sync_folders"] as JsonArray ?? []).DeepClone().AsArray(); next.RemoveAt(selected); _ = core.Apply(new() { ["sync_folders"] = next }); });
                remove.IsEnabled = !core.Running;
                pausedControls.Add(remove); folderControls.Add(remove); actions.Children.Add(remove); content.Children.Add(actions);
                rows.Children.Add(new Border { Child = content, Padding = new Thickness(16), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Gray) });
            }
        };
        refreshFolders();
        var add = Button("settings.add_folder", () => _ = AddFolder());
        add.IsEnabled = !core.Running; pausedControls.Add(add); panel.Children.Add(add);
        panel.Children.Add(Label("settings.stop_hint"));
        var deletes = new ToggleSwitch { Header = core.T("settings.deletes"), IsOn = core.Flag("propagate_deletes"), IsEnabled = !core.Running }; panel.Children.Add(deletes); pausedControls.Add(deletes);
        panel.Children.Add(Label("settings.bandwidth", 20)); Rate(panel, "settings.download_limit", "download_limit_kib"); Rate(panel, "settings.upload_limit", "upload_limit_kib");
        Switch(panel, "settings.pause_power", "pause_on_power_saver", true);
        var save = Button("settings.save", () => _ = core.Apply(new() { ["propagate_deletes"] = deletes.IsOn }));
        save.IsEnabled = !core.Running; panel.Children.Add(save); pausedControls.Add(save);
        if (core.Running) panel.Children.Add(Label("settings.pause_hint"));
    }
    private async Task AddFolder()
    {
        if (core.Running || settings is null) return;
        try
        {
            var picker = new global::Windows.Storage.Pickers.FolderPicker();
            picker.FileTypeFilter.Add("*");
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(settings));
            var folder = await picker.PickSingleFolderAsync();
            if (folder is null) return;
            var next = (core.Config["sync_folders"] as JsonArray ?? []).DeepClone().AsArray();
            next.Add(new JsonObject { ["local_folder"] = folder.Path, ["remote_path"] = "" });
            await core.Apply(new() { ["sync_folders"] = next });
        }
        catch (Exception exception) { if (pageError is not null) pageError.Text = exception.Message; }
    }
    private void OpenSelectedFolders()
    {
        var folders = core.Config["sync_folders"] as JsonArray ?? [];
        if (folders.Count == 1) Open(folders[0]!["local_folder"]!.ToString());
        else { ShowSettings(); if (navigation is not null) navigation.SelectedItem = navigation.MenuItems[0]; }
    }
    private void OpenSelectedRecovery()
    {
        var folders = core.Config["sync_folders"] as JsonArray ?? [];
        if (folders.Count == 1) Open(Path.Combine(folders[0]!["local_folder"]!.ToString(), ".filenode", "history"));
        else { ShowSettings(); if (navigation is not null) navigation.SelectedItem = navigation.MenuItems[0]; }
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
        var server = Field(panel, "settings.server", "server"); var web = Field(panel, "settings.web", "web_url"); var username = Field(panel, "settings.username", "username");
        var mode = new ComboBox { Header = core.T("settings.authentication"), ItemsSource = new[] { core.T("settings.auth_password"), core.T("settings.auth_browser") }, SelectedIndex = core.Text("auth_mode") == "browser" ? 1 : 0, IsEnabled = !core.Running };
        panel.Children.Add(mode); pausedControls.Add(mode);
        var credentials = new StackPanel { Spacing = 12 };
        var password = new PasswordBox { Header = core.T("settings.password"), PlaceholderText = core.T("settings.password_hint"), IsEnabled = !core.Running }; credentials.Children.Add(password); pausedControls.Add(password); credentials.Children.Add(Label("settings.credentials")); panel.Children.Add(credentials);
        var provider = new StackPanel { Spacing = 12 };
        provider.Children.Add(Label("settings.browser_hint"));
        var issuer = Field(provider, "settings.oauth_issuer", "oauth_issuer");
        issuer.PlaceholderText = "https://auth.example.org/application/o/mail/";
        var clientId = Field(provider, "settings.oauth_client", "oauth_client_id");
        var advanced = new Expander { Header = core.T("settings.oauth_advanced"), HorizontalAlignment = HorizontalAlignment.Stretch };
        var oauth = new StackPanel { Spacing = 12 };
        var scopes = Field(oauth, "settings.oauth_scopes", "oauth_scopes");
        var port = new NumberBox { Header = core.T("settings.oauth_port"), Minimum = 0, Maximum = 65535, Value = core.Number("oauth_redirect_port"), IsEnabled = !core.Running };
        oauth.Children.Add(port); pausedControls.Add(port);
        advanced.Content = oauth; provider.Children.Add(advanced); panel.Children.Add(provider);
        JsonObject Values() => new() { ["server"] = server.Text, ["web_url"] = web.Text, ["username"] = username.Text,
            ["auth_mode"] = mode.SelectedIndex == 1 ? "browser" : "app_password", ["oauth_issuer"] = issuer.Text,
            ["oauth_client_id"] = clientId.Text, ["oauth_scopes"] = scopes.Text, ["oauth_redirect_port"] = double.IsFinite(port.Value) ? (int)port.Value : 0 };
        accountActionLabel = () => core.Running ? "action.pause" : "action.signin";
        accountAction = Button(accountActionLabel(), () =>
        {
            if (core.Running) { _ = core.Send("pause"); return; }
            _ = core.Apply(Values(), true, mode.SelectedIndex == 1 ? null : password.Password, mode.SelectedIndex == 1); password.Password = "";
        });
        accountAction.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        accountAction.IsEnabled = core.Ready;
        void UpdateMode()
        {
            provider.Visibility = mode.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
            credentials.Visibility = mode.SelectedIndex == 1 ? Visibility.Collapsed : Visibility.Visible;
            accountAction.Content = core.T(accountActionLabel());
        }
        mode.SelectionChanged += (_, _) => UpdateMode(); UpdateMode();
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
    internal async Task RunSmokeTest(string directory)
    {
        App.SelfTestProgress("wait for worker");
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!core.Ready && DateTime.UtcNow < deadline) await Task.Delay(50);
        if (!core.Ready || core.Error != "") throw new InvalidOperationException("UI smoke test worker did not become ready: " + core.Error);
        App.SelfTestProgress("open status and settings");
        ShowStatus(); ShowSettings();
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
        }
        App.SelfTestProgress("stop one folder without deleting files");
        var remaining = core.Config["sync_folders"]!.DeepClone().AsArray(); remaining.RemoveAt(0);
        await core.Apply(new() { ["sync_folders"] = remaining });
        await WaitForFolders(1);
        if (!File.Exists(Path.Combine(first, "keep.txt")) || !Directory.Exists(second)) throw new InvalidOperationException("Stopping sync removed a selected folder");
        if (core.Running) throw new InvalidOperationException("Offline UI smoke test unexpectedly started synchronization");
        File.WriteAllText(Path.Combine(directory, "ui-smoke.json"), "{\"passed\":true,\"pages\":4,\"languages\":2,\"folders\":2,\"stopPreservesFiles\":true}");
        App.SelfTestProgress("quit");
        await Quit();
    }
}
