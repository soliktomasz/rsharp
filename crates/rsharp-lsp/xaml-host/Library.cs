using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.RegularExpressions;
using ICSharpCode.Decompiler;
using ICSharpCode.Decompiler.CSharp;
using ICSharpCode.Decompiler.TypeSystem;
using Passenger.Xaml;
using Passenger.Xaml.Features;
using Passenger.Xaml.Metadata;

namespace RSharp.XamlHost;

/// <summary>Where Go to Declaration leads into a library, as Passenger's LibrarySources, SourceLink and ThemePlaceAsync
/// find it (ported from its src/Passenger.App/Xaml/ and CSharp/CodeAssist.Xaml.cs): a library's type decompiled by ILSpy's
/// decompiler, and a theme's own XAML (Fluent's Button.xaml) downloaded through its PDB's Source Link, each written once
/// under <c>RSHARP_METADATA</c> (the IDE's state folder's MetadataAsSource/), read-only, so the editor shows it so.</summary>
internal static partial class Library
{
    private static readonly Guid SourceLinkKind = new("CC110556-A091-4D38-9FEC-25AB9A351A6A");

    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(20) };

    private static string Folder => Environment.GetEnvironmentVariable("RSHARP_METADATA") is { Length: > 0 } set
        ? set
        : Path.Combine(Path.GetTempPath(), "RSharp", "MetadataAsSource");

    /// <summary>The symbol server asked for PDBs; <c>RSHARP_SYMBOLS</c> names another, as tests point it nowhere.</summary>
    private static string Server => Environment.GetEnvironmentVariable("RSHARP_SYMBOLS") is { Length: > 0 } set
        ? set.TrimEnd('/') + "/"
        : "https://symbols.nuget.org/download/symbols/";

    /// <summary>A library type's decompiled file and where a member (or the type) is declared in it, as a line and a
    /// column from 0; null when its assembly cannot be read.</summary>
    public static (string Path, int Line, int Column)? Decompile(ClrType type, ClrMember? member)
    {
        if (type.Origin.AssemblyPath is not { } reference)
        {
            return null;
        }
        string assembly = Implementation(reference);
        string outermost = type.FullName.Split('+')[0];
        string folder = Path.Combine(Folder, "Decompiled", type.Origin.AssemblyName, Stamp(assembly));
        string path = Path.Combine(folder, outermost.Replace('`', '_') + ".cs");
        if (!File.Exists(path))
        {
            try
            {
                CSharpDecompiler decompiler = new(assembly, new DecompilerSettings { ThrowOnAssemblyResolveErrors = false });
                Written(path, System.Text.Encoding.UTF8.GetBytes(decompiler.DecompileTypeAsString(new FullTypeName(outermost))));
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException or BadImageFormatException or InvalidOperationException
                or ICSharpCode.Decompiler.Metadata.MetadataFileNotSupportedException)
            {
                return null;
            }
        }
        (int line, int column) = Declaration(File.ReadAllLines(path), member?.Name ?? type.Name,
            enumMember: member is { Kind: MemberKind.Field } && type.Kind == ClrTypeKind.Enum);
        return (path, line, column);
    }

    /// <summary>Where the theme library's own XAML writes what a target names (a control's theme, a part of its template, a
    /// resource the theme defines), from its Source Link; or why it cannot be had.</summary>
    public static async Task<((string Path, int Line, int Column)? Place, string Said)> ThemeAsync(XamlTarget target, XamlFile file)
    {
        AssemblyTypes? theme = file.Types.Assemblies.Where(a => a.Name.Contains(".Themes.", StringComparison.Ordinal) && a.XamlFiles.Count > 0)
            .OrderBy(a => a.Name.EndsWith(".Fluent", StringComparison.Ordinal) ? 0 : 1).FirstOrDefault();
        if (theme is null)
        {
            return (null, "The project references no theme whose XAML can be read");
        }
        string? why = null;
        string assembly = Implementation(theme.Path);
        List<string> known = [.. theme.XamlFiles];
        async Task<string?> Fetch(string path)
        {
            (string? local, string? failed) = await FileAsync(assembly, theme.Name, path);
            why = failed ?? why;
            return local;
        }
        IReadOnlyList<string> names = target.Type is { } type ? [.. file.Types.SelfAndBases(type).Select(t => t.Name)] : [];
        string? Named() => names.Select(n => known.FirstOrDefault(f => Path.GetFileNameWithoutExtension(f) == n)).FirstOrDefault(f => f is not null);
        bool Prefixed() => target.Part is { } key && known.Any(f => key.StartsWith(Path.GetFileNameWithoutExtension(f), StringComparison.Ordinal));
        if (target.Type is not null ? Named() is null : !Prefixed())
        {
            foreach (string hub in theme.XamlFiles.Where(f => f.EndsWith("Controls.xaml", StringComparison.Ordinal)))
            {
                if (await Fetch(hub) is { } local)
                {
                    known.AddRange(Included(File.ReadAllText(local), hub, theme.Name).Where(f => !known.Contains(f)));
                }
            }
        }
        List<string> candidates = [];
        if (target.Type is not null)
        {
            candidates.AddRange(Named() is { } named ? [named] : []);
        }
        else if (target.Part is { } key)
        {
            candidates.AddRange(known.Where(f => key.StartsWith(Path.GetFileNameWithoutExtension(f), StringComparison.Ordinal))
                .OrderByDescending(f => Path.GetFileNameWithoutExtension(f).Length));
            candidates.AddRange(known.Where(f => f.Contains("/Accents/", StringComparison.Ordinal) && !candidates.Contains(f)));
        }
        Regex place = ThemePlace(target);
        foreach (string candidate in candidates.Take(12))
        {
            if (await Fetch(candidate) is not { } local)
            {
                continue;
            }
            string text = File.ReadAllText(local);
            Match where = place.Match(text);
            if (where.Success)
            {
                int at = where.Groups["at"].Index;
                int line = text.AsSpan(0, at).Count('\n');
                return ((local, line, at - (text.LastIndexOf('\n', Math.Max(at - 1, 0)) + 1)), $"{theme.Name}'s {candidate.TrimStart('/')}, from its Source Link");
            }
        }
        return (null, why ?? $"{target.Name} is not in {theme.Name}'s XAML");
    }

    /// <summary>A file of the project an assembly was built from, by its path there, downloaded once; or why not.</summary>
    private static async Task<(string? Local, string? Why)> FileAsync(string assembly, string assemblyName, string path)
    {
        string folder = Path.Combine(Folder, "SourceLink", assemblyName, Stamp(assembly));
        string local = Path.Combine(folder, path.TrimStart('/').Replace('/', Path.DirectorySeparatorChar));
        if (File.Exists(local))
        {
            return (local, null);
        }
        try
        {
            if (await PdbAsync(assembly, folder) is not { } pdb)
            {
                return (null, $"{assemblyName} names no PDB on the symbol server");
            }
            (Dictionary<string, string> map, string? project) = Read(pdb, assemblyName);
            if (project is null || Map(map, project + path.TrimStart('/')) is not { } url)
            {
                return (null, $"{assemblyName}'s PDB has no Source Link for {path}");
            }
            if (!url.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
            {
                return (null, $"{assemblyName}'s Source Link is not HTTPS: {url}");
            }
            Written(local, await Http.GetByteArrayAsync(url));
            return (local, null);
        }
        catch (Exception error) when (error is HttpRequestException or IOException or UnauthorizedAccessException or BadImageFormatException or JsonException
            or TaskCanceledException)
        {
            return (null, $"{assemblyName}'s sources could not be downloaded: {error.Message}");
        }
    }

    /// <summary>The portable PDB an assembly names in its CodeView entry, downloaded once; null for one with none.</summary>
    private static async Task<string?> PdbAsync(string assembly, string folder)
    {
        CodeViewDebugDirectoryData codeView;
        List<string> checksums = [];
        using (FileStream stream = File.OpenRead(assembly))
        using (PEReader pe = new(stream))
        {
            IReadOnlyList<DebugDirectoryEntry> entries = pe.ReadDebugDirectory();
            DebugDirectoryEntry? entry = entries.Where(e => e.Type == DebugDirectoryEntryType.CodeView).Cast<DebugDirectoryEntry?>().FirstOrDefault();
            if (entry is not { } found)
            {
                return null;
            }
            codeView = pe.ReadCodeViewDebugDirectoryData(found);
            foreach (DebugDirectoryEntry checksum in entries.Where(e => e.Type == DebugDirectoryEntryType.PdbChecksum))
            {
                PdbChecksumDebugDirectoryData data = pe.ReadPdbChecksumDebugDirectoryData(checksum);
                checksums.Add($"{data.AlgorithmName}:{Convert.ToHexString(data.Checksum.AsSpan())}");
            }
        }
        string name = Path.GetFileName(codeView.Path.Replace('\\', '/'));
        string pdb = Path.Combine(folder, name);
        if (File.Exists(pdb))
        {
            return pdb;
        }
        // A portable PDB's key on a symbol server is its id with FFFFFFFF where a Windows PDB's age would be.
        string url = $"{Server}{Uri.EscapeDataString(name)}/{codeView.Guid:N}FFFFFFFF/{Uri.EscapeDataString(name)}";
        using HttpRequestMessage request = new(HttpMethod.Get, url);
        // NuGet's symbol server gives a portable PDB only to a request naming the checksum the assembly holds of it.
        if (checksums.Count > 0)
        {
            request.Headers.Add("SymbolChecksum", string.Join(';', checksums));
        }
        using HttpResponseMessage response = await Http.SendAsync(request);
        response.EnsureSuccessStatusCode();
        Directory.CreateDirectory(folder);
        await File.WriteAllBytesAsync(pdb, await response.Content.ReadAsByteArrayAsync());
        return pdb;
    }

    /// <summary>A PDB's Source Link map, and the folder of the project the assembly was built from: the one of its
    /// documents' folders named after the assembly, else their common folder.</summary>
    private static (Dictionary<string, string> Map, string? Project) Read(string pdb, string assemblyName)
    {
        using FileStream stream = File.OpenRead(pdb);
        using MetadataReaderProvider provider = MetadataReaderProvider.FromPortablePdbStream(stream);
        MetadataReader reader = provider.GetMetadataReader();
        Dictionary<string, string> map = new(StringComparer.Ordinal);
        foreach (CustomDebugInformationHandle handle in reader.GetCustomDebugInformation(EntityHandle.ModuleDefinition))
        {
            CustomDebugInformation information = reader.GetCustomDebugInformation(handle);
            if (reader.GetGuid(information.Kind) != SourceLinkKind)
            {
                continue;
            }
            using JsonDocument json = JsonDocument.Parse(reader.GetBlobBytes(information.Value));
            if (json.RootElement.TryGetProperty("documents", out JsonElement documents))
            {
                foreach (JsonProperty document in documents.EnumerateObject())
                {
                    map[document.Name.Replace('\\', '/')] = document.Value.GetString() ?? string.Empty;
                }
            }
        }
        List<string> folders = [.. reader.Documents.Select(d => reader.GetString(reader.GetDocument(d).Name).Replace('\\', '/'))
            .Select(p => p[..(p.LastIndexOf('/') + 1)])];
        string? project = folders.Where(f => f.EndsWith($"/{assemblyName}/", StringComparison.Ordinal)).OrderBy(f => f.Length).FirstOrDefault()
            ?? Common(folders);
        return (map, project);
    }

    private static string? Common(List<string> folders)
    {
        if (folders.Count == 0)
        {
            return null;
        }
        string common = folders[0];
        foreach (string folder in folders)
        {
            while (!folder.StartsWith(common, StringComparison.Ordinal))
            {
                common = common[..(common.TrimEnd('/').LastIndexOf('/') + 1)];
            }
        }
        return common.Length > 0 ? common : null;
    }

    /// <summary>A build path's URL by a Source Link map: an exact entry, else the longest pattern ending in * it starts
    /// with, the rest put in the *'s place.</summary>
    private static string? Map(Dictionary<string, string> map, string path)
    {
        if (map.TryGetValue(path, out string? exact))
        {
            return exact;
        }
        foreach ((string pattern, string url) in map.OrderByDescending(m => m.Key.Length))
        {
            if (pattern.EndsWith('*') && path.StartsWith(pattern[..^1], StringComparison.Ordinal) && url.Contains('*', StringComparison.Ordinal))
            {
                return url.Replace("*", path[(pattern.Length - 1)..], StringComparison.Ordinal);
            }
        }
        return null;
    }

    /// <summary>The files a theme's XAML merges in, as paths in its project: avares:// ones of its own assembly, and those
    /// beside the file.</summary>
    private static IEnumerable<string> Included(string text, string from, string assembly)
    {
        string folder = from[..(from.LastIndexOf('/') + 1)];
        foreach (Match include in IncludeSource().Matches(text))
        {
            string source = include.Groups["source"].Value;
            string own = $"avares://{assembly}/";
            if (source.StartsWith(own, StringComparison.Ordinal))
            {
                yield return "/" + source[own.Length..];
            }
            else if (!source.Contains("://", StringComparison.Ordinal))
            {
                yield return source.StartsWith('/') ? source : folder + source;
            }
        }
    }

    [GeneratedRegex(@"<(?:Merge)?ResourceInclude\b[^>]*\bSource=""(?<source>[^""]+)""")]
    private static partial Regex IncludeSource();

    /// <summary>Where a theme's XAML writes what a target names: a template part's name, a control's theme, a resource's key.</summary>
    private static Regex ThemePlace(XamlTarget target) => target switch
    {
        { Part: { } part, Type: not null } => new($@"\b(?:x:)?Name=""(?<at>{Regex.Escape(part)})"""),
        { Type: { } shown } => new($@"<(?<at>ControlTheme)\b[^>]*(?:\{{x:Type\s+{Regex.Escape(shown.Name)}\}}|TargetType=""{Regex.Escape(shown.Name)}"")"),
        { Part: { } key } => new($@"\bx:Key=""(?<at>{Regex.Escape(key)})"""),
        _ => new("(?!)"),
    };

    /// <summary>The assembly with the code: a package's lib/ beside the ref/ compiled against, the running runtime's for
    /// the framework's reference pack, else the one given.</summary>
    private static string Implementation(string reference)
    {
        string separator = Path.DirectorySeparatorChar.ToString();
        string refFolder = $"{separator}ref{separator}";
        if (reference.Contains(refFolder, StringComparison.Ordinal))
        {
            string lib = reference.Replace(refFolder, $"{separator}lib{separator}", StringComparison.Ordinal);
            if (File.Exists(lib))
            {
                return lib;
            }
        }
        if (reference.Contains($"{separator}packs{separator}", StringComparison.Ordinal))
        {
            string runtime = Path.Combine(RuntimeEnvironment.GetRuntimeDirectory(), Path.GetFileName(reference));
            if (File.Exists(runtime))
            {
                return runtime;
            }
        }
        return reference;
    }

    /// <summary>A folder name for one version of an assembly, so a package updated decompiles afresh.</summary>
    private static string Stamp(string assembly)
    {
        FileInfo file = new(assembly);
        return $"{(uint)HashCode.Combine(assembly.GetHashCode(StringComparison.Ordinal), file.LastWriteTimeUtc.Ticks, file.Length):x8}";
    }

    /// <summary>A file written whole and made read-only: it is the library's, which the editor shows and never saves.</summary>
    private static void Written(string path, byte[] bytes)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        string partial = path + ".part";
        File.WriteAllBytes(partial, bytes);
        File.Move(partial, path, overwrite: true);
        File.SetAttributes(path, File.GetAttributes(path) | FileAttributes.ReadOnly);
    }

    /// <summary>Where a name is declared in decompiled code: the first line declaring it with an access modifier (an enum's
    /// member, the line it starts), else its first mention, else the top.</summary>
    private static (int Line, int Column) Declaration(string[] lines, string name, bool enumMember)
    {
        Regex word = new($@"\b{Regex.Escape(name)}\b", RegexOptions.CultureInvariant);
        int mentioned = -1;
        for (int i = 0; i < lines.Length; i++)
        {
            Match found = word.Match(lines[i]);
            if (!found.Success)
            {
                continue;
            }
            string trimmed = lines[i].TrimStart();
            if (trimmed.StartsWith("//", StringComparison.Ordinal))
            {
                continue;
            }
            if ((enumMember && trimmed.StartsWith(name, StringComparison.Ordinal)) || (!enumMember && Modifier().IsMatch(trimmed)))
            {
                return (i, found.Index);
            }
            if (mentioned < 0)
            {
                mentioned = i;
            }
        }
        return mentioned < 0 ? (0, 0) : (mentioned, Math.Max(0, word.Match(lines[mentioned]).Index));
    }

    [GeneratedRegex(@"^(public|protected|internal)\b")]
    private static partial Regex Modifier();
}
