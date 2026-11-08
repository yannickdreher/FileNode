using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Microsoft.UI.Dispatching;

namespace FileNode.Windows;

internal sealed class FileActivity
{
    public required string Folder { get; init; }
    public required string Path { get; set; }
    public string State { get; set; } = "waiting";
    public string Operation { get; set; } = "check";
    public long Completed { get; set; }
    public long Total { get; set; }
    public ulong Limit { get; set; }
    public DateTimeOffset Updated { get; set; } = DateTimeOffset.Now;
    public bool Active => State == "transferring";
    public double Percent => Total > 0 ? Math.Clamp(100d * Completed / Total, 0, 100) : 0;
}

// A private inherited pipe, never a TCP listener. Only Rust handles account secrets and sync.
internal sealed class CoreClient : IAsyncDisposable
{
    private readonly DispatcherQueue dispatcher;
    private readonly SemaphoreSlim writer = new(1, 1);
    private readonly Dictionary<string, string> english;
    private readonly Dictionary<string, string> german;
    private Process? process;
    private bool stopping;
    public JsonObject Config { get; private set; } = new();
    public JsonObject Limits { get; private set; } = new();
    public string LimitsSummary {
        get {
            if (Limits.Count == 0) return T("settings.limits_unknown");
            var own = Unsigned(Config["max_file_bytes"]);
            var cap = Limits["server"]?["max_size_upload"] is JsonNode advertised ? Unsigned(advertised) : own == 0 ? 1073741824UL : own;
            if (own > 0) cap = Math.Min(cap, own);
            var count = Math.Min(Number("parallel_uploads", 2), (int)Math.Clamp(Unsigned(Limits["server"]?["max_concurrent_upload"], 1), 1UL, 16UL));
            return T("settings.limits_summary").Replace("{size}", FormatBytes(cap)).Replace("{count}", count.ToString());
        }
    }
    public static ulong Unsigned(JsonNode? value, ulong fallback = 0) => ulong.TryParse(value?.ToString(), out var number) ? number : fallback;
    public static string FormatBytes(ulong bytes) { var size = (double)bytes; var units = new[] { "B", "KiB", "MiB", "GiB", "TiB" }; int unit = 0; while (size >= 1024 && unit < units.Length - 1) { size /= 1024; unit++; } return $"{size:0.##} {units[unit]}"; }
    public List<FileActivity> Activity { get; } = [];
    public Dictionary<string, JsonObject> FolderStatuses { get; } = [];
    public string AccountState { get; private set; } = "signed_out";
    public bool Authenticated => AccountState == "signed_in";
    public bool PowerPaused { get; private set; }
    public string TrashUrl { get; private set; } = "";
    public string AccountStatus => T("account." + AccountState);
    public string FolderStatus(string folder) => T("folder." + (!Authenticated ? "not_connected" : !Running ? "paused" : PowerPaused ? "power_paused" : FolderStatuses.GetValueOrDefault(folder)?["state"]?.ToString() ?? "checking"));
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
        if (App.SelfTestDirectory is not null) start.Environment["FILENODE_LOG"] = "off";
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
    internal void Receive(JsonObject message)
    {
        var type = message["event"]?.ToString();
        bool configChanged = message["config"] is JsonObject;
        if (configChanged) Config = message["config"]!.AsObject().DeepClone().AsObject();
        switch (type)
        {
            case "ready":
                if (message["protocol"]?.GetValue<int>() != 4) { Error = "Incompatible Rust worker protocol"; break; }
                Ready = true;
                break;
            case "configured":
                Error = "";
                break;
            case "limits": Limits = message.DeepClone().AsObject(); break;
            case "file_limit":
                var limitedFolder = message["folder"]!.ToString(); var limitedPath = message["path"]!.ToString();
                var limited = Activity.Find(item => item.Folder == limitedFolder && item.Path == limitedPath);
                if (limited is null) { limited = new() { Folder = limitedFolder, Path = limitedPath }; Activity.Insert(0, limited); }
                limited.State = "too_large"; limited.Operation = "upload"; limited.Total = message["size"]!.GetValue<long>(); limited.Limit = Unsigned(message["limit"]);
                if (Activity.Count > 100) Activity.RemoveAt(100);
                break;
            case "open_browser":
                if (Uri.TryCreate(message["url"]?.ToString(), UriKind.Absolute, out var browserUrl) && browserUrl.Scheme == "https" && browserUrl.UserInfo == "")
                {
                    try { Process.Start(new ProcessStartInfo(browserUrl.AbsoluteUri) { UseShellExecute = true }); }
                    catch { Error = "The system browser could not open. Cancel and try again."; _ = Send("pause"); }
                }
                break;
            case "running": Running = message["value"]!.GetValue<bool>(); StatusKey = Running ? "status.connecting" : "status.paused"; break;
            case "account":
                AccountState = message["state"]!.ToString(); TrashUrl = message["trash_url"]?.ToString() ?? TrashUrl;
                if (AccountState == "signed_out") { Activity.Clear(); FolderStatuses.Clear(); TrashUrl = ""; Limits = new(); }
                if (AccountState == "failed") foreach (var pending in Activity.Where(item => item.Active || item.State == "retrying")) pending.State = "auth_required";
                break;
            case "folder":
                var verifiedFolder = message["folder"]!.ToString(); FolderStatuses[verifiedFolder] = message.DeepClone().AsObject();
                if (message["state"]!.ToString() == "current") foreach (var pending in Activity.Where(item => item.Folder == verifiedFolder && item.State is "waiting" or "checking" or "retrying" or "paused" or "auth_required")) { pending.State = "synced"; pending.Completed = pending.Total; pending.Updated = DateTimeOffset.Now; }
                break;
            case "renamed":
                var movedFolder = message["folder"]!.ToString(); var from = message["from"]!.ToString(); var to = message["to"]!.ToString();
                foreach (var moved in Activity.Where(item => item.Folder == movedFolder && (item.Path == from || item.Path.StartsWith(from + "/", StringComparison.Ordinal))).ToArray()) {
                    var destination = to + moved.Path[from.Length..];
                    Activity.RemoveAll(item => item != moved && item.Folder == movedFolder && item.Path == destination);
                    moved.Path = destination; moved.State = "checking"; moved.Operation = "rename";
                }
                break;
            case "file":
                var folder = message["folder"]!.ToString(); var path = message["path"]!.ToString();
                var item = Activity.Find(item => item.Folder == folder && item.Path == path);
                if (item is null) { item = new() { Folder = folder, Path = path }; Activity.Insert(0, item); }
                var state = message["state"]!.ToString();
                if (state != item.State) item.Updated = DateTimeOffset.Now;
                item.State = state; item.Operation = message["operation"]!.ToString();
                item.Completed = message["completed"]!.GetValue<long>(); item.Total = message["total"]!.GetValue<long>();
                if (item.Active) FolderStatuses[folder] = new() { ["state"] = "syncing" };
                if (Activity.Count > 100) Activity.RemoveAt(100);
                break;
            case "status": StatusKey = message["key"]!.ToString(); break;
            case "report":
                Error = ""; var report = message["report"]!;
                StatusKey = report["failed_folders"]?.GetValue<int>() > 0 ? "status.folder_error" : report["deferred"]!.GetValue<int>() > 0 ? "status.deferred" : report["blocked"]!.GetValue<int>() > 0 ? "status.deletions_blocked" : "status.current"; break;
            case "retry": case "error":
                if (AccountState == "checking") AccountState = "failed";
                Error = message["details"]?.ToString() ?? "Unknown Rust worker error"; StatusKey = "status.retry"; break;
            case "power": PowerPaused = message["paused"]!.GetValue<bool>(); if (PowerPaused) StatusKey = "status.power_paused"; break;
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
        await Send("configure", new() { ["patch"] = changes.DeepClone(), ["connect"] = connect, ["password"] = password ?? "", ["interactive"] = interactive });
    }
    public Task Power(bool saver, bool sleeping, bool available)
    {
        PowerAvailable = available;
        return Send("power", new() { ["saver"] = saver, ["sleeping"] = sleeping, ["available"] = available });
    }
    public async ValueTask DisposeAsync()
    {
        stopping = true;
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
