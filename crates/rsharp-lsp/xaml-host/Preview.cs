using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Text.Json.Nodes;
using Avalonia.Remote.Protocol;
using Avalonia.Remote.Protocol.Designer;
using Avalonia.Remote.Protocol.Viewport;
using Passenger.Xaml.Projects;

namespace RSharp.XamlHost;

/// <summary>An Avalonia file's preview, as Passenger's AvaloniaPreviewHost draws it: the host listens on loopback for the
/// project's Avalonia.Designer.HostApp, which RSharp starts on the project's last build (so it ends with the IDE), speaks
/// Avalonia.Remote.Protocol to it, and relays each frame and each XAML that did not load as a partial reply of the
/// <c>preview</c> request, which lasts until <c>preview_stop</c>.</summary>
internal static class Previews
{
    private static readonly ConcurrentDictionary<string, Session> Sessions = new(StringComparer.Ordinal);

    private sealed class Session(int request, string output, string projectPath)
    {
        public int Request { get; } = request;
        public string Output { get; } = output;
        public string ProjectPath { get; } = projectPath;
        public double Dpi { get; set; } = 96;
        public string Text { get; set; } = string.Empty;
        public IAvaloniaRemoteTransportConnection? Connection { get; set; }
        public IDisposable? Listening { get; set; }
        public TaskCompletionSource Ended { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        private readonly SemaphoreSlim sending = new(1, 1);

        /// <summary>One message at a time on the connection, as Passenger sends them all from its UI thread, without
        /// holding a thread while one goes: the host's requests share the thread pool with the connection.</summary>
        public async Task Send(object message)
        {
            await sending.WaitAsync();
            try
            {
                if (Connection is { } open)
                {
                    await open.Send(message);
                }
            }
            finally
            {
                sending.Release();
            }
        }
    }

    /// <summary>Starts a preview of the file as typed: the HostApp's command to start (a partial), then its frames and
    /// errors as partials until stopped; or why there is none (no build, no previewer in the package).</summary>
    public static async Task<JsonNode?> Run(int request, JsonNode asked)
    {
        string project = asked["project"]!.GetValue<string>();
        string path = asked["path"]!.GetValue<string>();
        if (Program.Builder(project) is not { } builder)
        {
            return new JsonObject { ["why"] = "Its project is neither Avalonia's nor WPF's" };
        }
        ProjectInputs inputs = builder.Current.Inputs;
        if (builder.Dialect is not Passenger.Xaml.Dialects.AvaloniaDialect)
        {
            // shortcut: no WPF previewer on Windows either; carry Passenger's WpfPreview there once WPF's previews are wanted.
            return new JsonObject
            {
                ["why"] = OperatingSystem.IsWindows() ? "RSharp previews Avalonia's XAML; WPF's preview is not in it yet" : "WPF XAML previews only on Windows, where WPF draws",
            };
        }
        if (Output(inputs) is not { } output)
        {
            return new JsonObject { ["why"] = $"Build {Path.GetFileNameWithoutExtension(inputs.ProjectPath)} to preview its XAML: the previewer draws its last build" };
        }
        if (HostApp(inputs) is not { } designer)
        {
            return new JsonObject { ["why"] = "The project's Avalonia package has no previewer (tools/…/designer/Avalonia.Designer.HostApp.dll)" };
        }
        string id = Guid.NewGuid().ToString("N");
        Session session = new(request, output, "/" + Path.GetRelativePath(inputs.Folder, path).Replace('\\', '/'))
        {
            Dpi = 96 * (asked["scale"]?.GetValue<double>() ?? 1),
            Text = asked["text"]!.GetValue<string>(),
        };
        Sessions[id] = session;
        int port = FreePort();
        session.Listening = new BsonTcpTransport().Listen(IPAddress.Loopback, port, connection => Connected(session, connection));
        string folder = Path.GetDirectoryName(output)!;
        string name = Path.GetFileNameWithoutExtension(output);
        JsonArray arguments =
        [
            "exec", "--runtimeconfig", Path.Combine(folder, name + ".runtimeconfig.json"), "--depsfile", Path.Combine(folder, name + ".deps.json"),
            designer, "--transport", $"tcp-bson://127.0.0.1:{port}/", "--method", "avalonia-remote", "--session-id", id, output,
        ];
        Partial(session, new JsonObject { ["started"] = new JsonObject { ["session"] = id, ["arguments"] = arguments, ["folder"] = folder } });
        await session.Ended.Task;
        session.Connection?.Dispose();
        session.Listening?.Dispose();
        Sessions.TryRemove(id, out _);
        return new JsonObject();
    }

    /// <summary>The file's text as typed, which the previewer loads again.</summary>
    public static JsonNode Update(JsonNode asked)
    {
        if (Sessions.TryGetValue(asked["session"]!.GetValue<string>(), out Session? session))
        {
            session.Text = asked["text"]!.GetValue<string>();
            Send(session);
        }
        return true;
    }

    public static JsonNode Stop(JsonNode asked)
    {
        if (Sessions.TryGetValue(asked["session"]!.GetValue<string>(), out Session? session))
        {
            session.Ended.TrySetResult();
        }
        return true;
    }

    private static void Connected(Session session, IAvaloniaRemoteTransportConnection connection)
    {
        if (session.Ended.Task.IsCompleted)
        {
            connection.Dispose();
            return;
        }
        session.Connection = connection;
        connection.OnMessage += (_, message) => Received(session, connection, message);
        connection.OnException += (_, error) => Partial(session, new JsonObject { ["error"] = $"The previewer's connection failed: {error.Message}" });
        connection.Start();
        _ = session.Send(new ClientSupportedPixelFormatsMessage { Formats = [PixelFormat.Bgra8888, PixelFormat.Rgba8888] });
        _ = session.Send(new ClientRenderInfoMessage { DpiX = session.Dpi, DpiY = session.Dpi });
        Send(session);
    }

    private static void Send(Session session)
    {
        _ = session.Send(new UpdateXamlMessage { Xaml = session.Text, AssemblyPath = session.Output, XamlFileProjectPath = session.ProjectPath });
    }

    private static void Received(Session session, IAvaloniaRemoteTransportConnection from, object message)
    {
        switch (message)
        {
            case RequestViewportResizeMessage resize:
                // Only said, never answered: an allocation in reply lays a root sized to its content out again, which asks
                // again, frame after frame (Passenger's finding).
                Partial(session, new JsonObject { ["empty"] = resize.Width < 1 || resize.Height < 1 });
                break;
            case UpdateXamlResultMessage result:
                Partial(session, new JsonObject { ["error"] = result.Error is { Length: > 0 } ? Describe(result) : null });
                break;
            case FrameMessage frame when frame.Width > 0 && frame.Height > 0 && frame.Data is not null:
                Partial(session, new JsonObject
                {
                    ["frame"] = new JsonObject
                    {
                        ["width"] = frame.Width,
                        ["height"] = frame.Height,
                        ["stride"] = frame.Stride,
                        ["rgba"] = frame.Format == PixelFormat.Rgba8888,
                        ["dpi"] = frame.DpiX,
                        ["data"] = Convert.ToBase64String(frame.Data),
                    },
                });
                _ = session.Send(new FrameReceivedMessage { SequenceId = frame.SequenceId });
                break;
        }
    }

    private static void Partial(Session session, JsonObject partial) =>
        Program.Write(new JsonObject { ["id"] = session.Request, ["partial"] = partial });

    /// <summary>What the host said of a XAML it could not load, with the line it names.</summary>
    private static string Describe(UpdateXamlResultMessage result)
    {
        if (result.Exception is { } details)
        {
            string where = details.LineNumber is { } line ? $" (line {line}{(details.LinePosition is { } column ? $", column {column}" : string.Empty)})" : string.Empty;
            return $"{details.ExceptionType}: {details.Message}{where}";
        }
        return result.Error.Split('\n')[0].Trim();
    }

    /// <summary>The project's newest build output that can run: its assembly with its runtime configuration and
    /// dependencies beside it, under its bin/ or an artifacts folder above it (Passenger's XamlPreview.Output).</summary>
    private static string? Output(ProjectInputs inputs)
    {
        string name = inputs.AssemblyName + ".dll";
        List<string> found = [];
        string bin = Path.Combine(inputs.Folder, "bin");
        if (Directory.Exists(bin))
        {
            found.AddRange(Directory.EnumerateFiles(bin, name, SearchOption.AllDirectories));
        }
        string project = Path.GetFileNameWithoutExtension(inputs.ProjectPath);
        for (DirectoryInfo? at = new(inputs.Folder); at is not null; at = at.Parent)
        {
            string artifacts = Path.Combine(at.FullName, "artifacts", "bin", project);
            if (Directory.Exists(artifacts))
            {
                found.AddRange(Directory.EnumerateFiles(artifacts, name, SearchOption.AllDirectories));
                break;
            }
        }
        return found.Where(f => File.Exists(Path.ChangeExtension(f, ".runtimeconfig.json")) && File.Exists(Path.ChangeExtension(f, ".deps.json"))
                && !f.Contains($"{Path.DirectorySeparatorChar}ref{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .OrderByDescending(File.GetLastWriteTimeUtc).FirstOrDefault();
    }

    /// <summary>Avalonia's previewer host in the project's Avalonia package: the newest of its tools/ frameworks that is not
    /// newer than the project's.</summary>
    private static string? HostApp(ProjectInputs inputs)
    {
        if (!inputs.PackageFolders.TryGetValue("Avalonia", out string? package) || !Directory.Exists(Path.Combine(package, "tools")))
        {
            return null;
        }
        Version target = Framework(inputs.TargetFramework) ?? new Version(99, 0);
        return Directory.EnumerateDirectories(Path.Combine(package, "tools"))
            .Select(d => (Folder: d, Version: Framework(Path.GetFileName(d))))
            .Where(d => d.Version is not null && d.Version <= target)
            .OrderByDescending(d => d.Version)
            .Select(d => Path.Combine(d.Folder, "designer", "Avalonia.Designer.HostApp.dll"))
            .FirstOrDefault(File.Exists);
    }

    private static Version? Framework(string? moniker) =>
        moniker is not null && moniker.StartsWith("net", StringComparison.Ordinal) && Version.TryParse(moniker[3..].Split('-')[0], out Version? version) && version.Major >= 5
            ? version
            : null;

    private static int FreePort()
    {
        TcpListener free = new(IPAddress.Loopback, 0);
        free.Start();
        int port = ((IPEndPoint)free.LocalEndpoint).Port;
        free.Stop();
        return port;
    }
}
