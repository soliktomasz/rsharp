using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Nodes;
using Passenger.WpfPreview;
using Passenger.Xaml.Projects;

namespace RSharp.XamlHost;

/// <summary>A WPF file's preview on Windows, as Passenger's WpfPreviewHost draws it (ported from its
/// src/Passenger.App/Xaml/WpfPreviewHost.cs): Passenger's own previewer (Passenger.WpfPreview, published into the host's
/// wpfpreview/ on Windows) run by the machine's dotnet on the project's runtime configuration (its own, rolling forward,
/// for a class library or a project before .NET 8), from a copy of the project's last build, so a build never meets its
/// assemblies locked. The host starts it itself, in the IDE's job, which ends it with the IDE; the text goes as JSON lines
/// with App.xaml's, and each PNG frame and its elements' boxes come back as the preview's partial replies.</summary>
internal static class WpfPreviews
{
    private static string Previewer => Path.Combine(AppContext.BaseDirectory, "wpfpreview", "Passenger.WpfPreview.dll");

    /// <summary>Where the builds previewed are copied, a folder per preview named for the host's process.</summary>
    private static string Copies => Path.Combine(Path.GetTempPath(), "RSharp", "previews");

    public static async Task<JsonNode?> Run(int request, JsonNode asked, XamlProjectBuilder builder, string path)
    {
        ProjectInputs inputs = builder.Current.Inputs;
        if (Previews.Output(inputs, configured: false) is not { } built)
        {
            return new JsonObject { ["why"] = $"Build {Path.GetFileNameWithoutExtension(inputs.ProjectPath)} to preview its XAML: the previewer draws its last build" };
        }
        if (!File.Exists(Previewer))
        {
            return new JsonObject { ["why"] = "WPF's previewer is missing from RSharp's sidecars (xaml/wpfpreview/)" };
        }
        string shadow = Path.Combine(Copies, $"{Environment.ProcessId}-{Guid.NewGuid():N}");
        string id = Guid.NewGuid().ToString("N");
        Previews.Session session = new(request, built, path) { Text = asked["text"]!.GetValue<string>() };
        double scale = asked["scale"]?.GetValue<double>() ?? 1;
        Process? previewer = null;
        try
        {
            await Task.Run(() => Copy(Path.GetDirectoryName(built)!, shadow));
            previewer = Start(inputs, Path.Combine(shadow, Path.GetFileName(built)), shadow);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or System.ComponentModel.Win32Exception)
        {
            Delete(shadow);
            return new JsonObject { ["why"] = $"The previewer could not start: {error.Message}" };
        }
        int sequence = 0;
        SemaphoreSlim writing = new(1, 1);
        async Task Send()
        {
            await writing.WaitAsync();
            try
            {
                session.Loaded = session.Text;
                string? application = builder.Current.Application?.Path;
                string? applicationText = application is null ? null : Program.TextOf(application, null);
                string line = JsonSerializer.Serialize(new WpfPreviewRequest
                {
                    Sequence = ++sequence,
                    Path = path,
                    Xaml = session.Text,
                    BaseUri = BaseUri(inputs, path),
                    Application = application is null || applicationText is null ? null : new WpfPreviewSource(application, applicationText, BaseUri(inputs, application)),
                    Scale = scale,
                }, WpfPreviewJson.Options);
                await previewer.StandardInput.WriteLineAsync(line);
                await previewer.StandardInput.FlushAsync();
            }
            catch (Exception error) when (error is IOException or ObjectDisposedException or InvalidOperationException)
            {
                // The previewer ended, which its reader says.
            }
            finally
            {
                writing.Release();
            }
        }
        session.Updated = () => _ = Send();
        Previews.Sessions[id] = session;
        Previews.Partial(session, new JsonObject { ["started"] = new JsonObject { ["session"] = id, ["arguments"] = new JsonArray(), ["folder"] = shadow } });
        _ = Task.Run(() => Read(session, previewer));
        await Send();
        await session.Ended.Task;
        Previews.Sessions.TryRemove(id, out _);
        try
        {
            previewer.Kill(entireProcessTree: true);
        }
        catch (InvalidOperationException)
        {
            // It ended already.
        }
        previewer.Dispose();
        Delete(shadow);
        return new JsonObject();
    }

    private static Process Start(ProjectInputs inputs, string assembly, string shadow)
    {
        string name = Path.GetFileNameWithoutExtension(assembly);
        string configuration = Path.Combine(shadow, name + ".runtimeconfig.json");
        // A class library has no runtime configuration, and one older than .NET 8 runs no previewer built for 8.
        bool own = !File.Exists(configuration) || Previews.Framework(inputs.TargetFramework) is not { Major: >= 8 };
        if (own)
        {
            configuration = Path.ChangeExtension(Previewer, ".runtimeconfig.json");
        }
        ProcessStartInfo start = new(Environment.ProcessPath!) { WorkingDirectory = shadow };
        foreach (string argument in (string[])["exec", "--runtimeconfig", configuration])
        {
            start.ArgumentList.Add(argument);
        }
        string dependencies = Path.Combine(shadow, name + ".deps.json");
        if (File.Exists(dependencies))
        {
            start.ArgumentList.Add("--depsfile");
            start.ArgumentList.Add(dependencies);
        }
        // A class library's build leaves its packages where restore put them, which its dependencies name as a cache does.
        foreach (string cache in inputs.PackageFolders.Values.Select(f => Path.GetDirectoryName(Path.GetDirectoryName(f))).OfType<string>().Distinct(StringComparer.Ordinal))
        {
            start.ArgumentList.Add("--additionalprobingpath");
            start.ArgumentList.Add(cache);
        }
        foreach (string argument in (string[])[Previewer, "--assembly", assembly])
        {
            start.ArgumentList.Add(argument);
        }
        start.Environment[WpfPreviewJson.SourceInfoVariable] = "1";
        if (own)
        {
            // Its own configuration rolls forward to the newest runtime, which a project on a preview of .NET needs.
            start.Environment["DOTNET_ROLL_FORWARD_TO_PRERELEASE"] = "1";
        }
        start.RedirectStandardInput = true;
        start.RedirectStandardOutput = true;
        start.RedirectStandardError = true;
        Process started = Process.Start(start) ?? throw new IOException("dotnet did not start");
        // Its standard error is its own log, which only matters once it ends.
        started.ErrorDataReceived += (_, _) => { };
        started.BeginErrorReadLine();
        return started;
    }

    /// <summary>The previewer's answers, each a frame (with its elements), the XAML's error, or why there is nothing.</summary>
    private static async Task Read(Previews.Session session, Process from)
    {
        try
        {
            while (await from.StandardOutput.ReadLineAsync() is { } line)
            {
                WpfPreviewReply? reply;
                try
                {
                    reply = JsonSerializer.Deserialize<WpfPreviewReply>(line, WpfPreviewJson.Options);
                }
                catch (JsonException)
                {
                    continue;
                }
                switch (reply)
                {
                    case { Kind: WpfPreviewKinds.Frame, Png: { } png }:
                        Previews.Partial(session, new JsonObject { ["error"] = null });
                        Previews.Partial(session, new JsonObject { ["png"] = new JsonObject { ["data"] = Convert.ToBase64String(png), ["dpi"] = 96 * reply.Scale } });
                        JsonArray drawn = [.. (reply.Elements ?? []).Select(b => (JsonNode)new JsonObject
                        {
                            ["Line"] = b.Line, ["Column"] = b.Column, ["X"] = b.X, ["Y"] = b.Y, ["Width"] = b.Width, ["Height"] = b.Height,
                        })];
                        Previews.Partial(session, new JsonObject { ["elements"] = Previews.ElementsOf(session, drawn) });
                        if (reply.Note is { } note)
                        {
                            Previews.Partial(session, new JsonObject { ["status"] = $"Live. {note}" });
                        }
                        break;
                    case { Kind: WpfPreviewKinds.Error }:
                        Previews.Partial(session, new JsonObject
                        {
                            ["error"] = reply.Line is { } at ? $"{reply.Message} (line {at}{(reply.Column is { } column ? $", column {column}" : string.Empty)})" : reply.Message,
                        });
                        break;
                    case { } nothing:
                        Previews.Partial(session, new JsonObject { ["status"] = nothing.Message ?? "Nothing to draw" });
                        break;
                }
            }
            Previews.Partial(session, new JsonObject { ["status"] = "The previewer ended" });
        }
        catch (Exception error) when (error is IOException or ObjectDisposedException or InvalidOperationException)
        {
            Previews.Partial(session, new JsonObject { ["status"] = $"The previewer ended: {error.Message}" });
        }
    }

    /// <summary>A file's pack URI in the project's assembly, which relative URIs in it resolve against as in the build.</summary>
    private static string BaseUri(ProjectInputs inputs, string path)
    {
        string relative = Path.GetRelativePath(inputs.Folder, path).Replace('\\', '/');
        if (relative.StartsWith("../", StringComparison.Ordinal))
        {
            relative = Path.GetFileName(path);
        }
        return $"pack://application:,,,/{Uri.EscapeDataString(inputs.AssemblyName)};component/{string.Join('/', relative.Split('/').Select(Uri.EscapeDataString))}";
    }

    /// <summary>A build's folder copied, its subfolders too, after copies a host no longer running left behind.</summary>
    private static void Copy(string from, string to)
    {
        if (Directory.Exists(Copies))
        {
            foreach (string folder in Directory.EnumerateDirectories(Copies))
            {
                string name = Path.GetFileName(folder);
                int dash = name.IndexOf('-', StringComparison.Ordinal);
                if (dash > 0 && int.TryParse(name[..dash], out int owner) && owner != Environment.ProcessId && !Running(owner))
                {
                    Delete(folder);
                }
            }
        }
        foreach (string file in Directory.EnumerateFiles(from, "*", SearchOption.AllDirectories))
        {
            string target = Path.Combine(to, Path.GetRelativePath(from, file));
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            File.Copy(file, target, overwrite: true);
        }
    }

    private static bool Running(int id)
    {
        try
        {
            using Process found = Process.GetProcessById(id);
            return !found.HasExited;
        }
        catch (Exception missing) when (missing is ArgumentException or InvalidOperationException)
        {
            return false;
        }
    }

    private static void Delete(string folder)
    {
        try
        {
            Directory.Delete(folder, recursive: true);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or DirectoryNotFoundException)
        {
            // Still held by a previewer as it ends: the next sweep takes it.
        }
    }
}
