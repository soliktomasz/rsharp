using System.Text;
using System.Text.Json.Nodes;
using System.Threading.Channels;
using Passenger.Profiler;

namespace RSharp.ProfilerHost;

/// <summary>Passenger.Profiler behind JSON lines on standard input and output, in the shape of the XAML host's: a first line
/// with id 0 says it is ready, then each request <c>{id, method, params}</c> is answered <c>{id, result}</c> or
/// <c>{id, error}</c>, requests running at once. One run at a time: <c>start</c> or <c>attach</c> begins it; <c>run</c>
/// streams the program's output as <c>partial</c> replies and answers with the snapshot when the program ends (or with
/// <c>stopped</c> once <c>stop</c> has taken the snapshot while the program goes on); <c>input</c> types to the program.</summary>
public static class Program
{
    /// <summary>The call tree's nodes under this share of the run's samples are left out: a deep tree of one-sample stacks
    /// is most of its size and none of what a flame graph can draw.</summary>
    // shortcut: the cut is fixed at 0.05% of the samples, so a run of millions of samples loses its rarest paths; make it a
    // request parameter when a person needs them.
    private const double Smallest = 0.0005;

    private static readonly object Writing = new();
    private static TextWriter protocol = TextWriter.Null;
    private static ProfilingSession? session;
    private static Channel<(string Text, bool Error)> output = Channel.CreateUnbounded<(string, bool)>();
    private static TaskCompletionSource<JsonNode> taken = new();
    private static readonly List<MemorySnapshot> Memory = [];

    public static int Main()
    {
        protocol = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(encoderShouldEmitUTF8Identifier: false)) { AutoFlush = true };
        Console.SetOut(Console.Error);
        Write(new JsonObject { ["id"] = 0, ["result"] = new JsonObject() });
        while (Console.In.ReadLine() is { } line)
        {
            if (line.Length > 0)
            {
                _ = Handle(line);
            }
        }
        session?.Kill();
        return 0;
    }

    private static async Task Handle(string line)
    {
        JsonNode? request = JsonNode.Parse(line);
        int id = request?["id"]?.GetValue<int>() ?? 0;
        JsonNode asked = request?["params"] ?? new JsonObject();
        try
        {
            JsonNode? result = await Run(request?["method"]?.GetValue<string>(), asked, id);
            Write(new JsonObject { ["id"] = id, ["result"] = result ?? new JsonObject() });
        }
        catch (Exception error)
        {
            Write(new JsonObject { ["id"] = id, ["error"] = error is ProfilingException or InvalidOperationException ? error.Message : error.ToString() });
        }
    }

    private static async Task<JsonNode?> Run(string? method, JsonNode asked, int id)
    {
        switch (method)
        {
            case "start":
                {
                    Begin();
                    string[] arguments = [.. asked["arguments"]!.AsArray().Select(a => a!.GetValue<string>())];
                    Dictionary<string, string> environment = asked["environment"]?.AsObject().ToDictionary(e => e.Key, e => e.Value!.GetValue<string>()) ?? [];
                    ProfileTarget target = new(asked["program"]!.GetValue<string>(), arguments, asked["workingDirectory"]!.GetValue<string>(), environment);
                    Channel<(string, bool)> lines = output;
                    session = await ProfilingSession.StartAsync(target, asked["folder"]!.GetValue<string>(), (text, error) => lines.Writer.TryWrite((text, error)));
                    return Started(target.Name);
                }
            case "attach":
                {
                    Begin();
                    string name = asked["name"]!.GetValue<string>();
                    session = await ProfilingSession.AttachAsync(asked["pid"]!.GetValue<int>(), name, asked["folder"]!.GetValue<string>());
                    return Started(name);
                }
            case "input":
                {
                    StreamWriter input = Current().Input ?? throw new InvalidOperationException("The program was attached to: its input is not the profiler's");
                    await input.WriteAsync(asked["text"]!.GetValue<string>());
                    await input.FlushAsync();
                    return null;
                }
            case "run":
                return await Follow(id);
            case "stop":
                {
                    Snapshot snapshot = await Current().StopAsync();
                    JsonNode json = Describe(snapshot);
                    taken.TrySetResult(json);
                    return json;
                }
            case "memory":
                {
                    MemorySnapshot snapshot = await Current().TakeMemorySnapshotAsync();
                    Memory.Add(snapshot);
                    return new JsonObject
                    {
                        ["number"] = snapshot.Number,
                        ["name"] = snapshot.Name,
                        ["taken"] = snapshot.Taken.ToString("HH:mm:ss"),
                        ["bytes"] = snapshot.Bytes,
                        ["objects"] = snapshot.Objects,
                        ["types"] = new JsonArray([.. snapshot.Types.OrderByDescending(t => t.Bytes).Take(300).Select(t => (JsonNode)new JsonObject
                        {
                            ["name"] = t.Name,
                            ["count"] = t.Count,
                            ["bytes"] = t.Bytes,
                            ["retained"] = t.RetainedBytes,
                        })]),
                    };
                }
            case "compare":
                {
                    MemorySnapshot older = Memory.First(m => m.Number == asked["older"]!.GetValue<int>());
                    MemorySnapshot newer = Memory.First(m => m.Number == asked["newer"]!.GetValue<int>());
                    if (older.WhyNotComparable(newer) is { } why)
                    {
                        throw new InvalidOperationException(why);
                    }
                    SnapshotComparison comparison = older.CompareWith(newer);
                    return new JsonObject
                    {
                        ["title"] = comparison.Title,
                        ["newObjects"] = comparison.NewObjects,
                        ["deadObjects"] = comparison.DeadObjects,
                        ["survivedObjects"] = comparison.SurvivedObjects,
                        ["inexact"] = comparison.Inexact,
                        ["types"] = new JsonArray([.. comparison.Types.OrderByDescending(t => Math.Abs(t.BytesDelta)).Take(300).Select(t => (JsonNode)new JsonObject
                        {
                            ["name"] = t.Name,
                            ["objectsDelta"] = t.ObjectsDelta,
                            ["bytesDelta"] = t.BytesDelta,
                            ["newObjects"] = t.NewObjects,
                            ["newBytes"] = t.NewBytes,
                            ["deadObjects"] = t.DeadObjects,
                            ["deadBytes"] = t.DeadBytes,
                        })]),
                    };
                }
            case "kill":
                Current().Kill();
                return null;
            default:
                throw new InvalidOperationException($"No method {method}");
        }
    }

    private static void Begin()
    {
        if (session is not null)
        {
            throw new InvalidOperationException("A run is under way: stop it first");
        }
        Memory.Clear();
        output = Channel.CreateUnbounded<(string, bool)>();
        taken = new TaskCompletionSource<JsonNode>();
    }

    private static ProfilingSession Current() => session ?? throw new InvalidOperationException("Nothing is being profiled");

    private static JsonNode Started(string name) => new JsonObject { ["pid"] = session!.ProcessId, ["trace"] = session.TracePath, ["name"] = name, ["attached"] = session.Attached };

    /// <summary>Streams the program's output until it ends, then answers with the snapshot of the run (or <c>stopped</c> when
    /// <c>stop</c> took it), and lets the session go.</summary>
    private static async Task<JsonNode> Follow(int id)
    {
        ProfilingSession running = Current();
        Channel<(string Text, bool Error)> lines = output;
        Task ended = Task.WhenAny(running.ExitedAsync(), taken.Task);
        while (true)
        {
            Task<bool> more = lines.Reader.WaitToReadAsync().AsTask();
            if (await Task.WhenAny(more, ended) == ended && !lines.Reader.TryPeek(out _))
            {
                break;
            }
            while (lines.Reader.TryRead(out (string Text, bool Error) line))
            {
                Write(new JsonObject { ["id"] = id, ["partial"] = new JsonObject { ["text"] = line.Text, ["error"] = line.Error } });
            }
        }
        JsonNode result;
        try
        {
            result = taken.Task.IsCompleted ? new JsonObject { ["stopped"] = true } : Describe(await running.WaitForExitAsync());
        }
        finally
        {
            session = null;
            await running.DisposeAsync();
        }
        return result;
    }

    /// <summary>A snapshot as the window draws it: the call tree of each thread, the hot spots, the run's size, its
    /// collections and what it allocated.</summary>
    private static JsonNode Describe(Snapshot snapshot)
    {
        int floor = (int)(snapshot.Samples * Smallest);
        JsonArray Tree(IEnumerable<CallNode> nodes) => [.. nodes.Where(n => n.Total > floor).Select(n => (JsonNode)new JsonObject
        {
            ["name"] = n.Name,
            ["module"] = n.Module,
            ["total"] = n.Total,
            ["self"] = n.Self,
            ["children"] = Tree(n.Ordered),
        })];
        return new JsonObject
        {
            ["samples"] = snapshot.Samples,
            ["sampleMs"] = snapshot.SampleMs,
            ["durationMs"] = snapshot.Duration.TotalMilliseconds,
            ["trace"] = snapshot.TracePath,
            ["threads"] = Tree(snapshot.Threads),
            ["hotSpots"] = new JsonArray([.. snapshot.HotSpots.OrderByDescending(h => h.Self).Take(200).Select(h => (JsonNode)new JsonObject
            {
                ["name"] = h.Name,
                ["module"] = h.Module,
                ["self"] = h.Self,
                ["total"] = h.Total,
            })]),
            ["uiFrames"] = new JsonArray([.. snapshot.UiFrames.Select(f => (JsonNode)new JsonObject
            {
                ["number"] = f.Number,
                ["startMs"] = f.StartMs,
                ["durationMs"] = f.DurationMs,
                ["layoutMs"] = f.LayoutMs,
                ["recordMs"] = f.RecordMs,
                ["compositeMs"] = f.CompositeMs,
            })]),
            ["collections"] = new JsonArray([.. snapshot.Collections.Select(c => (JsonNode)new JsonObject
            {
                ["number"] = c.Number,
                ["generation"] = c.Generation,
                ["reason"] = c.Reason,
                ["atMs"] = c.AtMs,
                ["pauseMs"] = c.PauseMs,
            })]),
            ["allocations"] = new JsonArray([.. snapshot.Allocations.Select(a => (JsonNode)new JsonObject
            {
                ["name"] = a.Name,
                ["bytes"] = a.Bytes,
                ["ticks"] = a.Ticks,
            })]),
        };
    }

    private static void Write(JsonNode message)
    {
        lock (Writing)
        {
            protocol.WriteLine(message.ToJsonString());
        }
    }
}
