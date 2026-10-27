using System.Collections.Concurrent;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Passenger.Xaml;
using Passenger.Xaml.Features;
using Passenger.Xaml.Projects;
using Passenger.Xaml.Syntax;

namespace RSharp.XamlHost;

/// <summary>Passenger.Xaml behind JSON lines on standard input and output, as Passenger.Host speaks: a first line with id
/// 0 says it is ready, then each request <c>{id, method, params}</c> is answered <c>{id, result}</c> or <c>{id, error}</c>,
/// in input order. Places are LSP's: lines and UTF-16 columns from zero. Each request carries the file's
/// text as typed and its project; a project's types are read once, on its first request, and read again once a file of it
/// changes on disk (<c>changed</c>).</summary>
public static class Program
{
    private static readonly object Writing = new();
    private static readonly ConcurrentDictionary<string, Lazy<XamlProjectBuilder?>> Builders = new(StringComparer.Ordinal);
    private static readonly ConcurrentDictionary<string, string> Texts = new(StringComparer.Ordinal);
    private static readonly string? DotnetRoot = Path.GetDirectoryName(Environment.ProcessPath);
    private static TextWriter protocol = TextWriter.Null;

    public static int Main()
    {
        Console.InputEncoding = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false);
        protocol = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(encoderShouldEmitUTF8Identifier: false)) { AutoFlush = true };
        Console.SetOut(Console.Error);
        Write(new JsonObject { ["id"] = 0, ["result"] = new JsonObject() });
        while (Console.In.ReadLine() is { } line)
        {
            if (line.Length > 0)
            {
                // A close must follow earlier reads, so a delayed read cannot bring discarded text back.
                Handle(line);
            }
        }
        return 0;
    }

    private static void Handle(string line)
    {
        JsonNode? request;
        try
        {
            request = JsonNode.Parse(line);
        }
        catch (JsonException error)
        {
            Console.Error.WriteLine($"RSharp.XamlHost: a request that does not read: {error.Message}");
            return;
        }
        int id = request?["id"]?.GetValue<int>() ?? 0;
        JsonNode asked = request?["params"] ?? new JsonObject();
        try
        {
            JsonNode? result = request?["method"]?.GetValue<string>() switch
            {
                "check" => Check(asked),
                "complete" => Complete(asked),
                "symbol" => Symbol(asked),
                "changed" => Changed(asked),
                "closed" => Closed(asked),
                var method => throw new InvalidOperationException($"the XAML host has no method {method}"),
            };
            Write(new JsonObject { ["id"] = id, ["result"] = result });
        }
        catch (Exception error)
        {
            Write(new JsonObject { ["id"] = id, ["error"] = error.Message });
        }
    }

    /// <summary>The file read against its project, or null for a project whose XAML Passenger.Xaml does not read.</summary>
    private static (XamlFile File, bool Changed)? Read(JsonNode asked, bool publish = false)
    {
        string project = asked["project"]!.GetValue<string>();
        string path = asked["path"]!.GetValue<string>();
        string text = asked["text"]!.GetValue<string>();
        Texts[path] = text;
        XamlProjectBuilder? builder = Builders.GetOrAdd(project, p => new Lazy<XamlProjectBuilder?>(() =>
        {
            XamlProjectBuilder? made = XamlProjectBuilder.For(p, DotnetRoot);
            made?.Load(Texts.GetValueOrDefault);
            return made;
        })).Value;
        if (builder is null)
        {
            return null;
        }
        XmlTree tree = XmlTree.Parse(text);
        // Only a check publishes the summary and reports its change to the other open files.
        XamlProject? changed = publish ? builder.XamlTyped(path, tree) : null;
        XamlProject current = changed ?? builder.Current;
        return (new XamlFile(path, tree, current.With(XamlFileSummary.Of(path, tree, current.ResolveSource))), changed is not null);
    }

    private static JsonNode Check(JsonNode asked)
    {
        if (Read(asked, publish: true) is not { File: var file, Changed: var changed })
        {
            return new JsonObject { ["dialect"] = null, ["diagnostics"] = new JsonArray() };
        }
        JsonArray found = [];
        foreach (XamlDiagnostic diagnostic in XamlDiagnostics.Of(file))
        {
            found.Add(new JsonObject
            {
                ["range"] = Range(file.Text, diagnostic.Range.Start, diagnostic.Range.End),
                ["error"] = diagnostic.Severity == XamlSeverity.Error,
                ["message"] = diagnostic.Message,
            });
        }
        return new JsonObject { ["dialect"] = file.Dialect.GetType().Name.Replace("Dialect", string.Empty, StringComparison.Ordinal), ["diagnostics"] = found, ["changed"] = changed };
    }

    private static JsonNode? Complete(JsonNode asked)
    {
        if (Read(asked) is not { File: var file } || XamlCompletion.At(file, Offset(file.Text, asked)) is not { } list)
        {
            return null;
        }
        JsonArray items = [];
        foreach (XamlItem item in list.Items)
        {
            items.Add(new JsonObject
            {
                ["label"] = item.Label,
                ["kind"] = item.Kind.ToString(),
                ["insert"] = item.Insert,
                ["snippet"] = item.IsSnippet,
                ["reopen"] = item.Reopen,
                ["detail"] = item.Detail,
                ["documentation"] = item.Documentation,
            });
        }
        return new JsonObject { ["start"] = Point(file.Text, list.Start), ["wordCharacters"] = list.WordCharacters, ["items"] = items };
    }

    private static JsonNode? Symbol(JsonNode asked)
    {
        if (Read(asked) is not { File: var file } || XamlNavigation.At(file, Offset(file.Text, asked)) is not { } symbol)
        {
            return null;
        }
        JsonArray targets = [];
        foreach (XamlTarget target in symbol.Targets)
        {
            // shortcut: a library's type (no source place) is not decompiled, nor a theme's XAML fetched through Source
            // Link; add Passenger's LibrarySources and SourceLink once Go to Declaration into Avalonia itself is wanted.
            (string Path, JsonNode Point)? place = target switch
            {
                { Place: { } at } => (at.Path, new JsonObject { ["line"] = at.Line, ["character"] = at.Column }),
                { Resource: { } resource } => (resource.Path, Point(TextOf(resource.Path, file), resource.KeyRange.Start)),
                { Path: { } other, Range: { } range } => (other, Point(TextOf(other, file), range.Start)),
                { Path: { } other } => (other, new JsonObject { ["line"] = 0, ["character"] = 0 }),
                { Kind: XamlTargetKind.Element, Range: { } range } => (file.Path, Point(file.Text, range.Start)),
                _ => null,
            };
            if (place is { } found)
            {
                targets.Add(new JsonObject { ["kind"] = target.Kind.ToString(), ["name"] = target.Name, ["path"] = found.Path, ["at"] = found.Point });
            }
        }
        return new JsonObject
        {
            ["range"] = Range(file.Text, symbol.Range.Start, symbol.Range.End),
            ["name"] = symbol.Name,
            ["description"] = XamlNavigation.Describe(file, symbol),
            ["targets"] = targets,
        };
    }

    /// <summary>Files written on disk: a C# or XAML file read again into the projects that hold it, and a project whose
    /// project file or restore changed read afresh on its next request.</summary>
    private static JsonNode Changed(JsonNode asked)
    {
        foreach (string path in asked["paths"]!.AsArray().Select(p => p!.GetValue<string>()))
        {
            Texts.TryRemove(path, out _);
            string name = Path.GetFileName(path);
            foreach ((string project, Lazy<XamlProjectBuilder?> lazy) in Builders)
            {
                if (name == "project.assets.json" || string.Equals(path, project, Paths.Comparison) || name.EndsWith(".props", StringComparison.OrdinalIgnoreCase))
                {
                    Builders.TryRemove(project, out _);
                }
                else if (lazy.IsValueCreated && lazy.Value is { } builder)
                {
                    if (path.EndsWith(".cs", StringComparison.OrdinalIgnoreCase))
                    {
                        builder.SourceChanged(path, null);
                    }
                    else if (builder.Owns(path))
                    {
                        builder.XamlChanged(path, null);
                    }
                }
            }
        }
        return true;
    }

    private static JsonNode Closed(JsonNode asked)
    {
        string path = asked["path"]!.GetValue<string>();
        Texts.TryRemove(path, out _);
        foreach (Lazy<XamlProjectBuilder?> lazy in Builders.Values)
        {
            if (lazy.IsValueCreated && lazy.Value is { } builder && builder.Owns(path))
            {
                builder.XamlChanged(path, null);
            }
        }
        return true;
    }

    /// <summary>Another file's text as the editor has it, else as it is on disk.</summary>
    private static string TextOf(string path, XamlFile file) =>
        path == file.Path ? file.Text : Texts.TryGetValue(path, out string? open) ? open : File.Exists(path) ? File.ReadAllText(path) : string.Empty;

    private static int Offset(string text, JsonNode asked)
    {
        int line = asked["line"]!.GetValue<int>();
        int at = 0;
        for (int i = 0; i < line && at >= 0; i++)
        {
            at = text.IndexOf('\n', at) is var next and >= 0 ? next + 1 : -1;
        }
        return at < 0 ? text.Length : Math.Min(at + asked["character"]!.GetValue<int>(), text.Length);
    }

    private static JsonObject Point(string text, int offset)
    {
        offset = Math.Clamp(offset, 0, text.Length);
        int lineStart = text.LastIndexOf('\n', Math.Max(offset - 1, 0)) is var at && at >= 0 && at < offset ? at + 1 : 0;
        int line = 0;
        for (int i = text.IndexOf('\n'); i >= 0 && i < lineStart; i = text.IndexOf('\n', i + 1))
        {
            line++;
        }
        return new JsonObject { ["line"] = line, ["character"] = offset - lineStart };
    }

    private static JsonObject Range(string text, int start, int end) => new() { ["start"] = Point(text, start), ["end"] = Point(text, end) };

    private static void Write(JsonObject reply)
    {
        string line = reply.ToJsonString();
        lock (Writing)
        {
            protocol.WriteLine(line);
        }
    }
}
