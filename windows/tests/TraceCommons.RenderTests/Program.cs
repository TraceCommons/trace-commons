using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Numerics;
using System.Reflection;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Geometry;
using TraceCommons.Interop.Shell;
using Windows.UI;

namespace TraceCommons.RenderTests;

/// <summary>Marks a render test. The harness runs every one it finds.</summary>
[AttributeUsage(AttributeTargets.Method)]
internal sealed class RenderTestAttribute : Attribute
{
}

/// <summary>
/// Runs the Win2D render tests offscreen and prints one line per test.
/// </summary>
/// <remarks>
/// <para>
/// Exit codes: 0 every test passed (or the session cannot render and the run
/// is not required), 1 a test failed, 2 the test list and the tests disagree,
/// 3 the session cannot render and TC_RENDER_TESTS_REQUIRED=1.
/// </para>
/// <para>
/// The first lines always say which session this is and whether it is
/// interactive: on a CI runner that log is the evidence for where these tests
/// can run.
/// </para>
/// </remarks>
public static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        string outDir = args.Length > 0 ? args[0] : Path.Combine(AppContext.BaseDirectory, "render-out");
        Directory.CreateDirectory(outDir);

        int session = Process.GetCurrentProcess().SessionId;
        bool required = Environment.GetEnvironmentVariable("TC_RENDER_TESTS_REQUIRED") == "1";
        Console.WriteLine($"render-tests: session={session} userInteractive={Environment.UserInteractive} required={required} utc={DateTime.UtcNow:o}");

        MethodInfo[] tests = typeof(RenderChecks).GetMethods(BindingFlags.Static | BindingFlags.Public)
            .Where(m => m.GetCustomAttribute<RenderTestAttribute>() is not null)
            .OrderBy(m => m.Name, StringComparer.Ordinal)
            .ToArray();
        string[] listed = File.ReadAllLines(Path.Combine(AppContext.BaseDirectory, "render-tests.txt"))
            .Select(line => line.Trim())
            .Where(line => line.Length > 0 && !line.StartsWith('#'))
            .OrderBy(line => line, StringComparer.Ordinal)
            .ToArray();
        string[] found = tests.Select(m => m.Name).ToArray();
        if (!listed.SequenceEqual(found))
        {
            Console.WriteLine("LIST MISMATCH: render-tests.txt names " + string.Join(", ", listed)
                + "; the harness found " + string.Join(", ", found));
            return 2;
        }

        if (session == 0)
        {
            Console.WriteLine(
                "SKIP all render tests: this process is in session 0, where Win2D cannot create a CanvasDevice "
                + "(spike S1: 0x887A0022 from CanvasDevice::InitializePrimaryOutput). Run them in a logged-on "
                + "desktop session (the test box's /IT task).");
            return required ? 3 : 0;
        }

        CanvasDevice device;
        try
        {
            device = new CanvasDevice();
        }
        catch (Exception e)
        {
            Console.WriteLine($"SKIP all render tests: CanvasDevice failed with 0x{e.HResult:X8} in session {session}.");
            return required ? 3 : 0;
        }

        int failed = 0;
        using (device)
        {
            foreach (MethodInfo test in tests)
            {
                try
                {
                    string detail = (string)test.Invoke(null, new object[] { device, outDir })!;
                    Console.WriteLine($"PASS {test.Name}: {detail}");
                }
                catch (TargetInvocationException e) when (e.InnerException is not null)
                {
                    failed++;
                    Console.WriteLine($"FAIL {test.Name}: {e.InnerException.Message}");
                }
            }
        }

        Console.WriteLine($"render-tests: {tests.Length - failed} passed, {failed} failed, {tests.Length} run");
        return failed == 0 ? 0 : 1;
    }
}

/// <summary>The render tests. Each returns its measured numbers or throws.</summary>
public static partial class RenderChecks
{
    /// <summary>
    /// The title bar's N, drawn with Win2D from <see cref="NearMarkPath.Unit"/>
    /// at 64 px, against the Mac's ImageRenderer render of
    /// <c>GlassBrandMark.path(at:size:)</c> at the same size
    /// (Fixtures/mac-n-mark-64.png, made by the W2 probe at 3372425f4; Mac
    /// coverage 0.5155). Compares coverage, so colour and the two
    /// rasterisers' anti-aliasing at the edges are what it cannot see.
    /// </summary>
    [RenderTest]
    public static string NearMarkMatchesTheMacsRender(CanvasDevice device, string outDir)
    {
        const int Side = 64;
        using var target = new CanvasRenderTarget(device, Side, Side, 96f);
        using (CanvasDrawingSession ds = target.CreateDrawingSession())
        {
            ds.Clear(Color.FromArgb(0, 0, 0, 0));
            using CanvasGeometry mark = Geometry(device, Side);
            ds.FillGeometry(mark, Color.FromArgb(255, 0, 0, 0));
        }

        byte[] ours = target.GetPixelBytes();
        Task.Run(async () => await target.SaveAsync(Path.Combine(outDir, "win-n-mark-64.png"))).GetAwaiter().GetResult();
        using CanvasBitmap mac = Task.Run(async () => await CanvasBitmap.LoadAsync(
            device, Path.Combine(AppContext.BaseDirectory, "Fixtures", "mac-n-mark-64.png"), 96f)).GetAwaiter().GetResult();
        if (mac.SizeInPixels.Width != Side || mac.SizeInPixels.Height != Side)
        {
            throw new InvalidOperationException($"the Mac render is {mac.SizeInPixels.Width}x{mac.SizeInPixels.Height}, not {Side}x{Side}");
        }

        byte[] theirs = mac.GetPixelBytes();
        double sumDiff = 0, coverage = 0;
        int both = 0, either = 0, max = 0;
        for (int i = 3; i < ours.Length; i += 4)
        {
            int a = ours[i], b = theirs[i];
            int d = Math.Abs(a - b);
            sumDiff += d;
            max = Math.Max(max, d);
            coverage += a / 255.0;
            bool inA = a >= 128, inB = b >= 128;
            both += inA && inB ? 1 : 0;
            either += inA || inB ? 1 : 0;
        }

        int pixels = Side * Side;
        double mean = sumDiff / pixels / 255.0;
        double iou = either == 0 ? 0 : (double)both / either;
        string numbers = $"coverage={coverage / pixels:F4} meanAlphaDiff={mean:F4} maxAlphaDiff={max} iou={iou:F4}";
        if (iou < 0.97 || mean > 0.03)
        {
            throw new InvalidOperationException(numbers + " (want iou >= 0.97, mean <= 0.03)");
        }

        return numbers;
    }

    /// <summary>The unit mark as a Win2D geometry <paramref name="side"/> across.</summary>
    public static CanvasGeometry Geometry(ICanvasResourceCreator device, float side)
    {
        using var builder = new CanvasPathBuilder(device);
        bool open = false;
        foreach (PathStep s in NearMarkPath.Unit)
        {
            Vector2 P(double x, double y) => new((float)(x * side), (float)(y * side));
            switch (s.Kind)
            {
                case PathStepKind.Move:
                    if (open)
                    {
                        builder.EndFigure(CanvasFigureLoop.Open);
                    }

                    builder.BeginFigure(P(s.X, s.Y));
                    open = true;
                    break;
                case PathStepKind.Line:
                    builder.AddLine(P(s.X, s.Y));
                    break;
                case PathStepKind.Cubic:
                    builder.AddCubicBezier(P(s.X1, s.Y1), P(s.X2, s.Y2), P(s.X, s.Y));
                    break;
                case PathStepKind.Close:
                    builder.EndFigure(CanvasFigureLoop.Closed);
                    open = false;
                    break;
            }
        }

        if (open)
        {
            builder.EndFigure(CanvasFigureLoop.Open);
        }

        return CanvasGeometry.CreatePath(builder);
    }
}
