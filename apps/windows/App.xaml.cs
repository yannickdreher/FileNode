using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using System.Runtime.InteropServices;

namespace FileNode.Windows;

public partial class App : Application
{
    private DesktopHost? host;
    internal static bool IsPackaged
    {
        get { uint length = 0; return GetCurrentPackageFullName(ref length, 0) == 122; }
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetCurrentPackageFullName(ref uint length, nint name);
    internal static string? SelfTestDirectory => Option("--self-test");
    private static string selfTestStage = "initialize resources";
    public App()
    {
        UnhandledException += (_, args) => RecordSelfTestFailure(args.Exception);
        AppDomain.CurrentDomain.UnhandledException += (_, args) =>
        { if (args.ExceptionObject is Exception exception) RecordSelfTestFailure(exception); };
        try { InitializeComponent(); }
        catch (Exception exception) { RecordSelfTestFailure(exception); throw; }
    }
    internal static void RecordSelfTestFailure(Exception exception)
    {
        if (SelfTestDirectory is not string directory) { Logger.Unhandled(exception); return; }
        Directory.CreateDirectory(directory);
        File.WriteAllText(Path.Combine(directory, "ui-smoke.json"), System.Text.Json.JsonSerializer.Serialize(new
        { passed = false, stage = selfTestStage, exception = exception.GetType().FullName, hresult = $"0x{exception.HResult:X8}", message = exception.Message, stack = exception.StackTrace }));
    }
    internal static void SelfTestProgress(string stage)
    {
        selfTestStage = stage;
        if (SelfTestDirectory is not string directory) return;
        Directory.CreateDirectory(directory);
        File.WriteAllText(Path.Combine(directory, "ui-stage.json"), System.Text.Json.JsonSerializer.Serialize(new { stage }));
    }
    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            // One desktop host and worker per profile. A second launch opens the existing UI.
            var profile = Option("--config");
            if (SelfTestDirectory is string directory)
            {
                Directory.CreateDirectory(directory);
                profile = Path.Combine(Path.GetFullPath(directory), "profile.toml");
                // The UI smoke test never opens a real account or changes login autostart.
                File.WriteAllText(profile, "username = \"\"\nlanguage = \"en\"\nautostart = false\n");
            }
            var identity = profile is null ? "default" : Path.GetFullPath(profile).ToUpperInvariant();
            var key = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(System.Text.Encoding.UTF8.GetBytes(identity)));
            var instance = AppInstance.FindOrRegisterForKey("FileNode-" + key);
            if (!instance.IsCurrent)
            {
                await instance.RedirectActivationToAsync(AppInstance.GetCurrent().GetActivatedEventArgs());
                Exit(); return;
            }
            instance.Activated += (_, _) => host?.Dispatch(() => host.ShowStatus());
            SelfTestProgress("create desktop host");
            host = new DesktopHost();
            var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
            SelfTestProgress("start worker");
            host.Start(Option("--core"), profile, Has("--background") || activation.Kind == ExtendedActivationKind.StartupTask, Has("--settings"));
            if (SelfTestDirectory is string testDirectory) await host.RunSmokeTest(testDirectory);
        }
        catch (Exception exception) { RecordSelfTestFailure(exception); throw; }
    }
    private static bool Has(string flag) => Environment.GetCommandLineArgs().Contains(flag);
    private static string? Option(string flag)
    {
        var args = Environment.GetCommandLineArgs(); var index = Array.IndexOf(args, flag);
        return index >= 0 && index + 1 < args.Length ? args[index + 1] : null;
    }
}
