using System.IO.Compression;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Headless.XUnit;
using Avalonia.Input;
using Avalonia.VisualTree;
using Passenger.App;
using Passenger.App.Explorer;
using Passenger.App.Templates;
using Passenger.App.ToolWindows;
using Passenger.Core;

namespace Passenger.Tests;

/// <summary>RSharp's Phase 4 acceptance, Passenger's side: New Solution (Console App "Shop"), two class libraries added
/// through Add, New Project, two references, and a package installed through the NuGet window, all through the window,
/// on SDK 10.0.101 and a local feed; the solution folder (bin and obj left out) copied to RSHARP_PARITY_OUT.</summary>
public sealed class RSharpParityTests : IDisposable
{
    private readonly string settings = TestFolders.Create("settings-");
    private readonly string place = TestFolders.Create("parity-");

    public void Dispose()
    {
        TestFolders.Delete(settings);
        TestFolders.Delete(place);
    }

    [AvaloniaFact]
    public void A_solution_grows_to_three_projects_with_references_and_a_package()
    {
        string output = Environment.GetEnvironmentVariable("RSHARP_PARITY_OUT") ?? throw new InvalidOperationException("RSHARP_PARITY_OUT is not set");
        // dotnet test's own SDK (Passenger's 11) names itself to its children; a person's Passenger has none of these,
        // so the folder's global.json picks the SDK, as it does for RSharp.
        foreach (string variable in new[] { "MSBuildSDKsPath", "MSBuildExtensionsPath", "MSBUILD_EXE_PATH", "MSBuildLoadMicrosoftTargetsReadOnly", "DOTNET_HOST_PATH" })
        {
            Environment.SetEnvironmentVariable(variable, null);
        }
        File.WriteAllText(Path.Combine(place, "global.json"), "{\n  \"sdk\": {\n    \"version\": \"10.0.101\",\n    \"rollForward\": \"disable\"\n  }\n}\n");
        File.WriteAllText(Path.Combine(place, "nuget.config"),
            "<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"local\" value=\"feed\" />\n  </packageSources>\n  <config>\n    <add key=\"globalPackagesFolder\" value=\"packages\" />\n  </config>\n</configuration>\n");
        Directory.CreateDirectory(Path.Combine(place, "feed"));
        Package("Demo.Core", "1.0.0");
        Package("Demo.Core", "2.0.0");

        MainWindow window = new() { Width = 1200, Height = 800, StateFolder = settings };
        window.Show();
        window.KeepLayout();
        window.KeepSettings();
        window.Start(LaunchOptions.Empty);
        Ui.Settle();

        // The dialog opens beside the last solution, reading that folder's templates: a seed folder in the pinned place
        // opened and closed makes it this one (RSharp's reads them again as Location changes; Passenger's do not).
        string seed = Path.Combine(place, "seed");
        Directory.CreateDirectory(seed);
        Task opening = window.OpenFolder(seed);
        Ui.WaitFor(() => opening.IsCompleted, "the seed folder open", TimeSpan.FromMinutes(1));
        window.CloseSolution();
        Ui.Settle();

        // 1. New Solution: Console App, Shop, the rest as it opens.
        Click(window, window.Welcome.NewSolutionButton);
        NewProjectDialog dialog = Templates(window);
        Assert.Equal("Console App", dialog.Chosen?.Name);
        dialog.Folder = place;
        Type(dialog, "Shop");
        Create(dialog);
        string folder = PathNames.Canonical(Path.Combine(place, "Shop"));
        Ui.WaitFor(() => window.WorkspaceModel?.Projects.Count == 1, "the new solution open", TimeSpan.FromMinutes(2));

        // 2 and 3. Add, New Project on the solution: two class libraries.
        foreach (string name in new[] { "Shop.Core", "Shop.Data" })
        {
            RightClick(window, Row(window, NodeKind.Solution, null));
            ChooseFromMenu(window, "Add", "New Project...");
            NewProjectDialog adding = Templates(window);
            Type(adding, name);
            Click(adding, adding.FamilyRow("Class Library") ?? throw new InvalidOperationException("New Project lists no Class Library"));
            Assert.Equal(name, adding.ProjectName);
            Create(adding);
            Ui.WaitFor(() => window.WorkspaceModel!.Projects.Any(p => p.Name == name), $"{name} in the solution", TimeSpan.FromMinutes(2));
        }

        // 4. Add, Reference: Shop to Shop.Core, Shop.Core to Shop.Data.
        foreach ((string from, string to) in new[] { ("Shop", "Shop.Core"), ("Shop.Core", "Shop.Data") })
        {
            RightClick(window, Row(window, NodeKind.Project, from));
            ChooseFromMenu(window, "Add", "Reference...");
            AddReferenceDialog references = Assert.Single(window.OwnedWindows.OfType<AddReferenceDialog>());
            ReferenceChoice chosen = references.Choices.Single(c => c.Name == to);
            chosen.IsChosen = true;
            references.KeyPress(Key.Enter, RawInputModifiers.None, PhysicalKey.None, null);
            Ui.Settle();
            Assert.Contains(to, File.ReadAllText(Path.Combine(folder, from, $"{from}.csproj")), StringComparison.Ordinal);
        }

        // 5. The NuGet window: Demo.Core 2.0.0 installed in Shop.Core.
        Ui.WaitFor(() => window.Projects.Count == 3, "the three projects evaluated", TimeSpan.FromMinutes(2));
        NuGetView nuget = window.NuGet;
        nuget.Query = "Demo.Core";
        Task searched = nuget.RefreshAsync();
        Ui.Landed(() => searched.IsCompleted && nuget.Versions.Count == 2, "the package and its versions");
        nuget.ChosenVersion = "2.0.0";
        Task<bool> installed = nuget.InstallAsync(window.Projects.Single(p => p.Name == "Shop.Core"));
        Ui.Landed(() => installed.IsCompleted, "dotnet add package's end");
        Assert.True(installed.Result, nuget.Log);
        window.Close();

        if (Directory.Exists(output))
        {
            Directory.Delete(output, true);
        }
        foreach (string file in Directory.EnumerateFiles(folder, "*", SearchOption.AllDirectories))
        {
            string relative = Path.GetRelativePath(folder, file);
            if (relative.Split(Path.DirectorySeparatorChar).Any(part => part is "bin" or "obj"))
            {
                continue;
            }
            string target = Path.Combine(output, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            File.Copy(file, target);
        }
    }

    /// <summary>A package with nothing but its manifest and an empty lib folder, so any framework takes it.</summary>
    private void Package(string id, string version)
    {
        using ZipArchive zip = ZipFile.Open(Path.Combine(place, "feed", $"{id}.{version}.nupkg"), ZipArchiveMode.Create);
        using (StreamWriter spec = new(zip.CreateEntry($"{id}.nuspec").Open()))
        {
            spec.Write($"<?xml version=\"1.0\"?><package xmlns=\"http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd\"><metadata><id>{id}</id><version>{version}</version><authors>Folk</authors><description>The {id} package</description></metadata></package>");
        }
        zip.CreateEntry("lib/netstandard2.0/_._").Open().Dispose();
    }

    private static void Create(NewProjectDialog dialog)
    {
        Click(dialog, dialog.CreateButton);
        Ui.WaitFor(() => !dialog.IsVisible || dialog.Problem is not null, "dotnet new", TimeSpan.FromMinutes(5));
        Assert.True(dialog.Problem is null, $"dotnet new failed: {dialog.Problem}");
    }

    private static NewProjectDialog Templates(MainWindow window)
    {
        Ui.Settle();
        NewProjectDialog dialog = Assert.Single(window.OwnedWindows.OfType<NewProjectDialog>());
        Ui.WaitFor(() => dialog.TemplatesRead.IsCompleted, "dotnet new's templates to be read", TimeSpan.FromMinutes(3));
        return dialog;
    }

    private static ExplorerNode Row(MainWindow window, NodeKind kind, string? name) =>
        window.ExplorerView.Tree.Rows.FirstOrDefault(r => r.Kind == kind && (name is null || r.Name == name))
        ?? throw new InvalidOperationException($"The Explorer shows no {kind} {name}: [{string.Join(", ", window.ExplorerView.Tree.Rows.Select(r => r.Name))}]");

    private static void RightClick(MainWindow window, ExplorerNode row)
    {
        ListBox rows = window.ExplorerView.GetVisualDescendants().OfType<ListBox>().First(l => ReferenceEquals(l.ItemsSource, window.ExplorerView.Tree.Rows));
        rows.ScrollIntoView(row);
        Ui.Settle();
        Control item = rows.ContainerFromItem(row) ?? throw new InvalidOperationException($"The Explorer does not show {row.Name}");
        Point at = item.TranslatePoint(new Point(item.Bounds.Width / 2, item.Bounds.Height / 2), window)!.Value;
        window.MouseDown(at, MouseButton.Right);
        window.MouseUp(at, MouseButton.Right);
        Ui.Settle();
    }

    private static void ChooseFromMenu(MainWindow window, string submenu, string item)
    {
        ContextMenu menu = window.ExplorerView.OpenMenu ?? throw new InvalidOperationException("No menu opened on the row");
        MenuItem parent = menu.Items.OfType<MenuItem>().First(i => (string?)i.Header == submenu);
        MenuItem chosen = parent.Items.OfType<MenuItem>().First(i => (string?)i.Header == item);
        chosen.RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(MenuItem.ClickEvent));
        menu.Close();
        Ui.Settle();
    }

    private static void Type(TopLevel window, string text)
    {
        window.KeyTextInput(text);
        Ui.Settle();
    }

    private static void Click(TopLevel window, Control control)
    {
        Point at = control.TranslatePoint(new Point(Math.Min(12, control.Bounds.Width / 2), control.Bounds.Height / 2), window)
            ?? throw new InvalidOperationException("The control is not in the window");
        window.MouseDown(at, MouseButton.Left);
        window.MouseUp(at, MouseButton.Left);
        Ui.Settle();
    }
}
