using System.Collections.Concurrent;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis.Text;
using Passenger.Text;
using Passenger.Xaml;
using Passenger.Xaml.Dialects;
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
    private static readonly JsonSerializerOptions StyleNames = new(JsonSerializerDefaults.Web) { Converters = { new System.Text.Json.Serialization.JsonStringEnumConverter() } };
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
                // A close must follow earlier reads, so a delayed read cannot bring discarded text back: each request runs
                // here in turn, and only a preview, which awaits its end, gives the loop back before it is answered.
                _ = Handle(line);
            }
        }
        return 0;
    }

    private static async Task Handle(string line)
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
            // A preview lasts until it is stopped, its frames sent as it goes: awaited, it holds no thread meanwhile.
            JsonNode? result = request?["method"]?.GetValue<string>() == "preview" ? await Previews.Run(id, asked) : request?["method"]?.GetValue<string>() switch
            {
                "check" => Check(asked),
                "complete" => Complete(asked),
                "symbol" => Symbol(asked),
                "element" => Element(asked),
                "fixes" => Fixes(asked),
                "format" => Format(asked),
                "surround" => Surround(asked),
                "extract" => Extract(asked),
                "colours" => ColoursOf(asked),
                "recolour" => Recolour(asked),
                "preview_update" => Previews.Update(asked),
                "preview_stop" => Previews.Stop(asked),
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
        if (Builder(project) is not { } builder)
        {
            return null;
        }
        XmlTree tree = XmlTree.Parse(text);
        // Only a check publishes the summary and reports its change to the other open files.
        XamlProject? changed = publish ? builder.XamlTyped(path, tree) : null;
        XamlProject current = changed ?? builder.Current;
        return (new XamlFile(path, tree, current.With(XamlFileSummary.Of(path, tree, current.ResolveSource))), changed is not null);
    }

    /// <summary>A project's XAML, read on its first request; null for a project whose XAML Passenger.Xaml does not read.</summary>
    internal static XamlProjectBuilder? Builder(string project) => Builders.GetOrAdd(project, p => new Lazy<XamlProjectBuilder?>(() =>
    {
        XamlProjectBuilder? made = XamlProjectBuilder.For(p, DotnetRoot);
        made?.Load(Texts.GetValueOrDefault);
        return made;
    })).Value;

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
        JsonObject answer = new()
        {
            ["range"] = Range(file.Text, symbol.Range.Start, symbol.Range.End),
            ["name"] = symbol.Name,
            ["description"] = XamlNavigation.Describe(file, symbol),
            ["targets"] = targets,
        };
        // An element's name: where the file names it, and the class's C# files, where its code-behind field is used.
        if (symbol.Named is { } named)
        {
            JsonArray uses = [.. XamlNavigation.NameUses(file, named.Value).Select(r => (JsonNode)Range(file.Text, r.Start, r.End))];
            IEnumerable<string> code = file.ClassName is { } className ? file.Types.PlacesOf(className).Select(p => p.Path) : [];
            JsonArray files = [.. code.Append(file.Path + ".cs").Where(File.Exists).Distinct(StringComparer.Ordinal).Select(f => (JsonNode)f)];
            answer["named"] = new JsonObject { ["name"] = named.Value, ["uses"] = uses, ["files"] = files };
        }
        return answer;
    }

    /// <summary>The fixes of the problems on a line that edit an existing XAML file, as LSP code actions.</summary>
    /// <para>Each file's edits are made from <c>texts</c>' copy of it where the editor sent one (an open file with unsaved
    /// edits, C# included), else from the host's or the disk's.</para>
    private static JsonNode Fixes(JsonNode asked)
    {
        JsonArray actions = [];
        if (Read(asked) is not { File: var file })
        {
            return actions;
        }
        int line = asked["line"]!.GetValue<int>();
        foreach (XamlDiagnostic diagnostic in XamlDiagnostics.Of(file).Where(d => Point(file.Text, d.Range.Start)["line"]!.GetValue<int>() == line))
        {
            foreach (XamlFix fix in diagnostic.Fixes)
            {
                actions.Add(new JsonObject
                {
                    ["title"] = fix.Title,
                    ["kind"] = "quickfix",
                    ["edit"] = WorkspaceEdit(fix.Others.Prepend(new XamlFileChange(fix.Path, fix.Edits)), file, asked["texts"]?.AsObject()),
                });
            }
        }
        return actions;
    }

    /// <summary>Reformat Code: Rider's XAML code style at its defaults with the file's .editorconfig over it, the file's
    /// own indent and line breaks, only the lines of a range when given; or why it was left as it is.</summary>
    private static JsonNode Format(JsonNode asked)
    {
        string text = asked["text"]!.GetValue<string>();
        JsonObject config = asked["editorconfig"]!.AsObject();
        // Settings' Code Style: XAML, by XamlFormatSettings' own names; Rider's defaults where none is sent.
        XamlFormatSettings style = asked["settings"]?.Deserialize<XamlFormatSettings>(StyleNames) ?? new XamlFormatSettings();
        XamlFormatSettings settings = style.Clamped().Over(key => config[key]?.GetValue<string>());
        IndentOptions indent = new(asked["indentSize"]!.GetValue<int>(), asked["tabs"]!.GetValue<bool>());
        TextRange? within = asked["range"] is { } range
            ? TextRange.FromBounds(Offset(text, range["start"]!), Offset(text, range["end"]!))
            : null;
        XamlFormatted formatted = XamlFormatter.Format(text, settings, indent, asked["eol"]!.GetValue<string>(), within);
        JsonArray edits = [.. formatted.Edits.Select(e => (JsonNode)new JsonObject { ["range"] = Range(text, e.Start, e.End), ["newText"] = e.Text })];
        return new JsonObject { ["edits"] = edits, ["problem"] = formatted.Problem };
    }

    /// <summary>What Surround With wraps: the element at a caret, or the elements a selection touches under one parent,
    /// and how many; null where none can be wrapped.</summary>
    private static JsonNode? Surround(JsonNode asked)
    {
        string text = asked["text"]!.GetValue<string>();
        TextRange selection = TextRange.FromBounds(Offset(text, asked["start"]!), Offset(text, asked["end"]!));
        if (XamlRefactorings.SurroundTarget(XmlTree.Parse(text), selection) is not { } target)
        {
            return null;
        }
        JsonObject found = Range(text, target.Range.Start, target.Range.End);
        found["count"] = target.Count;
        return found;
    }

    /// <summary>Extract Resource or Extract Style at a caret, by <c>kind</c>: without <c>name</c>, what is there (or why
    /// nothing is) and where it can go; with it, why that name will not do, or with <c>make</c> the changes as a workspace
    /// edit.</summary>
    private static JsonNode? Extract(JsonNode asked)
    {
        if (Read(asked) is not { File: var file })
        {
            return new JsonObject { ["why"] = "The project's XAML is not read" };
        }
        bool style = asked["kind"]!.GetValue<string>() == "style";
        int offset = Offset(file.Text, asked);
        string? name = asked["name"]?.GetValue<string>();
        bool make = asked["make"]?.GetValue<bool>() == true;
        IReadOnlyList<ResourcePlace> places = XamlRefactorings.Places(file, style && file.Dialect is AvaloniaDialect, p => TextOf(p, file));
        if (style)
        {
            (StyleSource? source, string? why) = XamlRefactorings.StyleAt(file, offset);
            if (source is null)
            {
                return new JsonObject { ["why"] = why };
            }
            if (name is null)
            {
                JsonArray properties = [.. source.Properties.Select(p => (JsonNode)new JsonObject { ["name"] = p.Name, ["written"] = p.Written, ["binding"] = p.IsBinding })];
                return new JsonObject { ["suggested"] = source.Suggested, ["properties"] = properties, ["places"] = Titles(places) };
            }
            string? problem = XamlRefactorings.StyleNameProblem(file, source, name);
            if (problem is not null || !make)
            {
                return new JsonObject { ["problem"] = problem };
            }
            List<string> chosen = [.. asked["chosen"]!.AsArray().Select(c => c!.GetValue<string>())];
            ResourcePlace place = places[asked["place"]!.GetValue<int>()];
            return new JsonObject { ["edit"] = WorkspaceEdit(XamlRefactorings.ExtractStyle(file, source, name, chosen, place), file), ["title"] = place.Title };
        }
        (ExtractableValue? value, string? whyNot) = XamlRefactorings.ValueAt(file, offset);
        if (value is null)
        {
            return new JsonObject { ["why"] = whyNot };
        }
        if (name is null)
        {
            return new JsonObject { ["suggested"] = value.Suggested, ["others"] = value.Others, ["places"] = Titles(places) };
        }
        string? keyProblem = XamlRefactorings.KeyProblem(file, name);
        if (keyProblem is not null || !make)
        {
            return new JsonObject { ["problem"] = keyProblem };
        }
        bool everywhere = asked["everywhere"]!.GetValue<bool>();
        ResourcePlace into = places[asked["place"]!.GetValue<int>()];
        IReadOnlyList<XamlFileChange> changes = XamlRefactorings.ExtractResource(file, value, name, into, everywhere);
        return new JsonObject { ["edit"] = WorkspaceEdit(changes, file), ["title"] = into.Title, ["uses"] = everywhere ? value.Uses.Count : 1 };
    }

    /// <summary>The colours a XAML file (its theme <c>variant</c>'s, a resource's) or a C# file writes or names: each one's
    /// line, its colour, where a colour chosen for it is written (a file and a range in its text, none for one nothing
    /// can change in place) and the use itself, which <c>recolour</c> is given back.</summary>
    private static JsonNode ColoursOf(JsonNode asked)
    {
        string path = asked["path"]!.GetValue<string>();
        string text = asked["text"]!.GetValue<string>();
        IReadOnlyList<ColourUse> uses;
        if (path.EndsWith(".cs", StringComparison.OrdinalIgnoreCase))
        {
            uses = Colours.InCSharp(Colours.Parse(text));
        }
        else if (Read(asked) is { File: var file })
        {
            uses = Colours.InXaml(file, asked["variant"]?.GetValue<string>());
        }
        else
        {
            return new JsonArray();
        }
        JsonArray found = [];
        Dictionary<string, SourceText> texts = new(StringComparer.Ordinal) { [path] = SourceText.From(text) };
        foreach (ColourUse use in uses)
        {
            string target = use.WritePath ?? path;
            if (!texts.TryGetValue(target, out SourceText? written))
            {
                written = SourceText.From(TextOf(target, null));
                texts[target] = written;
            }
            JsonObject colour = new()
            {
                ["line"] = texts[path].Lines.GetLinePosition(use.Offset).Line,
                ["colour"] = $"#{use.Colour.A:X2}{use.Colour.R:X2}{use.Colour.G:X2}{use.Colour.B:X2}",
                ["use"] = new JsonObject { ["form"] = use.Form.ToString(), ["receiver"] = use.Receiver },
            };
            if (use.WriteRange is { } range && range.End <= written.Length)
            {
                colour["path"] = target;
                colour["range"] = new JsonObject { ["start"] = Point(written, range.Start), ["end"] = Point(written, range.End) };
                colour["use"]!["written"] = written.ToString(TextSpan.FromBounds(range.Start, range.End));
            }
            found.Add(colour);
        }
        return found;
    }

    /// <summary>A colour written as the use it replaces was written (a name, <c>#RRGGBB</c>, a call's arguments, a hex
    /// number).</summary>
    private static JsonNode? Recolour(JsonNode asked)
    {
        JsonNode use = asked["use"]!;
        if (KnownColours.Parse(asked["colour"]!.GetValue<string>()) is not { } colour)
        {
            return null;
        }
        ColourUse made = new(0, colour, null, null, Enum.Parse<ColourForm>(use["form"]!.GetValue<string>()), use["receiver"]!.GetValue<string>());
        return Colours.Written(made, colour, use["written"]?.GetValue<string>() ?? string.Empty);
    }

    private static JsonArray Titles(IReadOnlyList<ResourcePlace> places) => [.. places.Select(p => (JsonNode)p.Title)];

    /// <summary>XAML changes as LSP's workspace edit: each file's edits made from its text as it stands (an open file's as
    /// typed), a file made as a create and its text.</summary>
    private static JsonObject WorkspaceEdit(IEnumerable<XamlFileChange> changes, XamlFile file, JsonObject? texts = null)
    {
        JsonArray made = [];
        foreach (XamlFileChange change in changes)
        {
            string uri = new Uri(change.Path).AbsoluteUri;
            if (change.Made is not null)
            {
                made.Add(new JsonObject { ["kind"] = "create", ["uri"] = uri });
                made.Add(Edits(uri, string.Empty, [new TextEdit(0, 0, change.Made)]));
            }
            if (change.Edits is not null)
            {
                string text = texts?[change.Path]?.GetValue<string>() ?? TextOf(change.Path, file);
                made.Add(Edits(uri, text, change.Edits(text)));
            }
        }
        return new JsonObject { ["documentChanges"] = made };
    }

    private static JsonObject Edits(string uri, string text, IEnumerable<TextEdit> edits) => new()
    {
        ["textDocument"] = new JsonObject { ["uri"] = uri },
        ["edits"] = new JsonArray([.. edits.Select(e => (JsonNode)new JsonObject { ["range"] = Range(text, e.Start, e.End), ["newText"] = e.Text })]),
    };

    /// <summary>Where the XAML of a class names an element: what the field Avalonia's generator makes for it leads to.</summary>
    private static JsonNode? Element(JsonNode asked)
    {
        string className = asked["class"]!.GetValue<string>();
        string name = asked["name"]!.GetValue<string>();
        if (Builder(asked["project"]!.GetValue<string>()) is not { } builder)
        {
            return null;
        }
        foreach (XamlFileSummary file in builder.Current.Files.Values.Where(f => f.Class == className))
        {
            if (file.Names.FirstOrDefault(n => n.Name == name) is { Name: not null } named)
            {
                return new JsonObject { ["path"] = file.Path, ["at"] = Point(TextOf(file.Path, null), named.Range.Start) };
            }
        }
        return null;
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
    private static string TextOf(string path, XamlFile? file) =>
        path == file?.Path ? file.Text : Texts.TryGetValue(path, out string? open) ? open : File.Exists(path) ? File.ReadAllText(path) : string.Empty;

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

    internal static JsonObject Range(string text, int start, int end) => new() { ["start"] = Point(text, start), ["end"] = Point(text, end) };

    private static JsonObject Point(SourceText text, int offset)
    {
        LinePosition at = text.Lines.GetLinePosition(offset);
        return new JsonObject { ["line"] = at.Line, ["character"] = at.Character };
    }

    internal static void Write(JsonObject reply)
    {
        string line = reply.ToJsonString();
        lock (Writing)
        {
            protocol.WriteLine(line);
        }
    }
}
