using System.Net.Sockets;
using System.Reflection;
using System.Runtime.ExceptionServices;
using System.Text.Json;
using Avalonia;
using Avalonia.Controls;
using Avalonia.DesignerSupport.Remote;
using Avalonia.Markup.Xaml;
using Avalonia.Markup.Xaml.Diagnostics;
using Avalonia.VisualTree;

namespace RSharp.Preview;

/// <summary>Avalonia's previewer as Avalonia.Designer.HostApp's own Main runs it, its XAML loader wrapped so it records
/// where each element is written (CreateSourceInfo), and each design window's elements, with their boxes in the window,
/// sent to the XAML host as a JSON line once laid out: what a click in the preview chooses and the caret outlines.
/// Arguments: the HostApp, the XAML host's port, then the HostApp's own.</summary>
public static class Program
{
    private static StreamWriter? sending;
    private static Window? shown;
    private static string? sent;

    public static void Main(string[] args)
    {
        Assembly hostApp = Assembly.LoadFrom(args[0]);
        // The loader interface is Avalonia's internal one, which only a proxy can implement from here.
        Type loader = typeof(AvaloniaXamlLoader).GetNestedType("IRuntimeXamlLoader", BindingFlags.NonPublic)!;
        object recording = DispatchProxy.Create(loader, typeof(SourceInfoLoader));
        ((SourceInfoLoader)recording).Inner = Activator.CreateInstance(hostApp.GetType("Avalonia.Designer.HostApp.DesignXamlLoader", true)!, nonPublic: true)!;
        // The locator's binding is Avalonia's [PrivateApi], out of its reference assemblies, as the HostApp binds it.
        object locator = typeof(AvaloniaLocator).GetProperty("CurrentMutable")!.GetValue(null)!;
        object binding = locator.GetType().GetMethod("Bind")!.MakeGenericMethod(loader).Invoke(locator, null)!;
        MethodInfo constant = binding.GetType().GetMethod("ToConstant")!;
        (constant.IsGenericMethodDefinition ? constant.MakeGenericMethod(loader) : constant).Invoke(binding, [recording]);

        TcpClient client = new("127.0.0.1", int.Parse(args[1], System.Globalization.CultureInfo.InvariantCulture));
        sending = new StreamWriter(client.GetStream()) { AutoFlush = true };
        Window.WindowOpenedEvent.AddClassHandler<Window>((window, _) => Follow(window));
        RemoteDesignerEntryPoint.Main(args[2..]);
    }

    /// <summary>The design window the previewer opened for the XAML last sent; the one before is closed.</summary>
    private static void Follow(Window window)
    {
        shown = window;
        sent = null;
        window.LayoutUpdated += (_, _) => Send(window);
    }

    /// <summary>Every element of the file with a place in its XAML, parents before children, as the window lays it out
    /// now; sent only when it differs from what was sent last. The file's root has no place of its own (the loader makes
    /// it before reading its markup), so it goes first as line 0: the window, or the control a plain design window
    /// holds.</summary>
    private static void Send(Window window)
    {
        if (window != shown || sending is null)
        {
            return;
        }
        List<Drawn> drawn = [];
        Visual? root = window.GetType() == typeof(Window) ? window.Content as Visual : window;
        foreach (Visual visual in window.GetVisualDescendants().Prepend(window))
        {
            if (!visual.IsEffectivelyVisible || visual.TranslatePoint(default, window) is not { } at)
            {
                continue;
            }
            // The file the previewer was sent is loaded as runtimexaml; a place elsewhere is a theme's or a style's.
            if (XamlSourceInfo.GetXamlSourceInfo((AvaloniaObject)visual) is { SourceUri.Scheme: "file" } info
                && info.SourceUri.Host.StartsWith("runtimexaml", StringComparison.Ordinal))
            {
                drawn.Add(new Drawn(info.LineNumber, info.LinePosition, at.X, at.Y, visual.Bounds.Width, visual.Bounds.Height));
            }
            else if (visual == root)
            {
                drawn.Add(new Drawn(0, 0, at.X, at.Y, visual.Bounds.Width, visual.Bounds.Height));
            }
        }
        string line = JsonSerializer.Serialize(drawn);
        if (line != sent)
        {
            sent = line;
            try
            {
                sending.WriteLine(line);
            }
            catch (IOException)
            {
                sending = null;
            }
        }
    }

    private sealed record Drawn(int Line, int Column, double X, double Y, double Width, double Height);
}

/// <summary>The HostApp's loader, asked to record where each element is written.</summary>
public class SourceInfoLoader : DispatchProxy
{
    public object? Inner { get; set; }

    protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
    {
        if (args is [_, RuntimeXamlLoaderConfiguration configuration])
        {
            configuration.CreateSourceInfo = true;
        }
        try
        {
            return targetMethod!.Invoke(Inner, args);
        }
        catch (TargetInvocationException thrown) when (thrown.InnerException is { } error)
        {
            // The previewer says the XAML's own error, with its line, as it would without the wrapper.
            ExceptionDispatchInfo.Throw(error);
            throw;
        }
    }
}
