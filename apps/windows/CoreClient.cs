using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Microsoft.UI.Dispatching;

namespace FileNode.Windows;

// A private inherited pipe, never a TCP listener. Only Rust handles account secrets and sync.
internal sealed class CoreClient : IAsyncDisposable
{
    private readonly DispatcherQueue dispatcher;
    private readonly SemaphoreSlim writer = new(1, 1);
    private readonly Dictionary<string, string> english;
    private readonly Dictionary<string, string> german;
    private Process? process;
    private bool stopping;
    private string? pendingPassword;
    private bool connectAfterSave;
    private bool pendingInteractive;
    public JsonObject Config { get; private set; } = new();
    public List<JsonObject> Activity { get; } = [];
    public bool Ready { get; private set; }
    public bool Running { get; private set; }
    public bool PowerAvailable { get; private set; }
    public string StatusKey { get; private set; } = "status.ready";
    public string Error { get; private set; } = "";
    public event Action<bool>? Changed;
    public string Language => Text("language", "auto") is var code && code != "auto"
        ? code : CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "de" ? "de" : "en";

    public CoreClient(DispatcherQueue dispatcher)
    {
        this.dispatcher = dispatcher;
        english = ReadCatalog("en"); german = ReadCatalog("de");
    }
    private static Dictionary<string, string> ReadCatalog(string code) =>
        System.Text.Json.JsonSerializer.Deserialize<Dictionary<string, string>>(
            File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "locales", code + ".json")))!;
    public string T(string key) => (Language == "de" ? german : english).GetValueOrDefault(key, english.GetValueOrDefault(key, key));
    public string Text(string key, string fallback = "") => Config[key]?.GetValue<string>() ?? fallback;
    public bool Flag(string key, bool fallback = false) => Config[key]?.GetValue<bool>() ?? fallback;
    public int Number(string key, int fallback = 0) => Config[key]?.GetValue<int>() ?? fallback;
    public string Format(JsonObject message)
    {
        var args = new Dictionary<string, string>();
        if (message["arguments"] is JsonArray array)
            foreach (var item in array.OfType<JsonArray>())
                if (item.Count == 2) args[item[0]!.ToString()] = item[1]!.ToString();
        return Regex.Replace(T(message["key"]!.ToString()), @"\{([^{}]+)\}", match => args.GetValueOrDefault(match.Groups[1].Value, match.Value));
    }
    public void Start(string? corePath, string? profile)
    {
        var start = new ProcessStartInfo(corePath ?? Path.Combine(AppContext.BaseDirectory, "filenode-core.exe"))
        { UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true, RedirectStandardOutput = true, RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8 };
        start.ArgumentList.Add("--service");
        // Packaged startup is handled through StartupTask, not a versioned package path in HKCU.
        if (!App.IsPackaged) { start.ArgumentList.Add("--gui-executable"); start.ArgumentList.Add(Environment.ProcessPath!); }
        else start.ArgumentList.Add("--external-autostart");
        if (profile is not null) { start.ArgumentList.Add("--config"); start.ArgumentList.Add(profile); }
        try
        {
            process = Process.Start(start) ?? throw new IOException("Rust worker could not start");
            _ = ReadEvents(process);
            _ = DrainErrors(process);
        }
        catch (Exception error) { Error = error.Message; Changed?.Invoke(false); }
    }
    private async Task DrainErrors(Process worker)
    {
        // Do not display raw child stderr: startup paths can contain personal information.
        while (await worker.StandardError.ReadLineAsync() is not null) { }
    }
    private async Task ReadEvents(Process worker)
    {
        try
        {
            // Bound frames before allocation. Credentials never travel on this output channel.
            var buffer = new char[4096]; var frame = new StringBuilder();
            int count;
            while ((count = await worker.StandardOutput.ReadAsync(buffer)) > 0)
                for (int index = 0; index < count; index++)
                {
                    if (buffer[index] != '\n')
                    {
                        frame.Append(buffer[index]);
                        if (frame.Length > 16 * 1024 * 1024) throw new IOException("Rust response exceeds frame limit");
                        continue;
                    }
                    var message = JsonNode.Parse(frame.ToString())!.AsObject(); frame.Clear();
                    dispatcher.TryEnqueue(() => Receive(message));
                }
            await worker.WaitForExitAsync();
            if (!stopping) dispatcher.TryEnqueue(() => { Running = false; Ready = false; Error = "Rust worker exited. Restart FileNode to reconnect."; Changed?.Invoke(false); });
        }
        catch (Exception error) { if (!stopping) dispatcher.TryEnqueue(() => { Error = error.Message; Running = false; Changed?.Invoke(false); }); }
    }
    private void Receive(JsonObject message)
    {
        var type = message["event"]?.ToString();
        bool configChanged = message["config"] is JsonObject;
        if (configChanged) Config = message["config"]!.AsObject().DeepClone().AsObject();
        switch (type)
        {
            case "ready":
                if (message["protocol"]?.GetValue<int>() != 2) { Error = "Incompatible Rust worker protocol"; break; }
                Ready = true;
                break;
            case "configured":
                if (connectAfterSave) { var password = pendingPassword; pendingPassword = null; connectAfterSave = false; _ = Send("connect", new() { ["password"] = password ?? "", ["interactive"] = pendingInteractive }); pendingInteractive = false; }
                break;
            case "open_browser":
                if (Uri.TryCreate(message["url"]?.ToString(), UriKind.Absolute, out var browserUrl) && browserUrl.Scheme == "https" && browserUrl.UserInfo == "")
                {
                    try { Process.Start(new ProcessStartInfo(browserUrl.AbsoluteUri) { UseShellExecute = true }); }
                    catch { Error = "The system browser could not open. Cancel and try again."; _ = Send("pause"); }
                }
                break;
            case "running": Running = message["value"]!.GetValue<bool>(); StatusKey = Running ? "status.connecting" : "status.paused"; break;
            case "status": StatusKey = message["key"]!.ToString(); break;
            case "report":
                Error = ""; var report = message["report"]!;
                StatusKey = report["deferred"]!.GetValue<int>() > 0 ? "status.deferred" : report["blocked"]!.GetValue<int>() > 0 ? "status.deletions_blocked" : "status.current"; break;
            case "activity":
                message["time"] = DateTimeOffset.Now.ToString("g"); Activity.Insert(0, message);
                if (Activity.Count > 100) Activity.RemoveAt(100); break;
            case "retry": case "error":
                pendingPassword = null; connectAfterSave = false; Error = message["details"]?.ToString() ?? "Unknown Rust worker error"; StatusKey = "status.retry"; break;
            case "power": if (message["paused"]!.GetValue<bool>()) StatusKey = "status.power_paused"; break;
        }
        Changed?.Invoke(configChanged);
        // The host publishes initial power state while handling ready, before auto-connect.
        if (type == "ready" && Ready && Text("username") != "") _ = Send("connect");
    }
    public async Task Send(string command, JsonObject? data = null)
    {
        if (process is null || process.HasExited) return;
        data ??= new(); data["command"] = command;
        await writer.WaitAsync();
        try { await process.StandardInput.WriteLineAsync(data.ToJsonString()); await process.StandardInput.FlushAsync(); }
        catch (IOException error) { Error = error.Message; Changed?.Invoke(false); }
        finally { writer.Release(); }
    }
    public async Task Apply(JsonObject changes, bool connect = false, string? password = null, bool interactive = false)
    {
        var next = Config.DeepClone().AsObject();
        foreach (var pair in changes) next[pair.Key] = pair.Value?.DeepClone();
        connectAfterSave = connect; pendingPassword = password; pendingInteractive = interactive;
        await Send("configure", new() { ["config"] = next });
    }
    public Task Power(bool saver, bool sleeping, bool available)
    {
        PowerAvailable = available;
        return Send("power", new() { ["saver"] = saver, ["sleeping"] = sleeping, ["available"] = available });
    }
    public async ValueTask DisposeAsync()
    {
        stopping = true; pendingPassword = null;
        if (process is { HasExited: false })
        {
            await Send("quit");
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
            try { await process.WaitForExitAsync(timeout.Token); }
            catch (OperationCanceledException) { process.Kill(entireProcessTree: true); }
        }
        process?.Dispose(); writer.Dispose();
    }
}
