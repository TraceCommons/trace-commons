using System;
using System.IO;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The shell's crash log: what an exception that escapes the UI thread leaves
/// behind, so the next crash names its throwing line. Payload-free (no
/// message, no argument values, no directories), once per exception type, and
/// never a second failure.
/// </summary>
public sealed class CrashLogTests : IDisposable
{
    private readonly string _dir = Path.Combine(Path.GetTempPath(), "tc-crashlog-" + Guid.NewGuid().ToString("N"));
    private static readonly DateTimeOffset Now = new(2026, 11, 22, 13, 0, 0, TimeSpan.Zero);

    public void Dispose()
    {
        try { Directory.Delete(_dir, true); } catch (IOException) { }
    }

    private static Exception Thrown(Func<Exception> make)
    {
        try { throw make(); } catch (Exception e) { return e; }
    }

    private static Exception Thrower() => Thrown(() => new InvalidOperationException("secret prompt text C:\\Users\\alice\\notes.txt"));

    [Fact]
    public void AnEntryNamesTheTypeTheHResultAndTheFrames()
    {
        var e = Thrower();
        string entry = CrashLog.Format("first-run", e, Now);
        Assert.Contains("2026-11-22T13:00:00Z", entry);
        Assert.Contains("where=first-run", entry);
        Assert.Contains("type=System.InvalidOperationException", entry);
        Assert.Contains("hresult=0x80131509", entry);
        Assert.Contains(nameof(Thrower), entry);
    }

    [Fact]
    public void NoMessageAndNoDirectoryReachesTheLog()
    {
        string entry = CrashLog.Format("xaml", Thrower(), Now);
        Assert.DoesNotContain("secret prompt text", entry);
        Assert.DoesNotContain("alice", entry);
        Assert.DoesNotContain("Users", entry);
        Assert.DoesNotContain(Path.DirectorySeparatorChar + "notes", entry);
    }

    [Theory]
    [InlineData(@"   at TraceCommons.App.FirstRunWindow.Render() in C:\src\repo\windows\src\TraceCommons.App\FirstRunWindow.xaml.cs:line 659",
        "at TraceCommons.App.FirstRunWindow.Render FirstRunWindow.xaml.cs:line 659")]
    [InlineData("   at A.B.C(String secret, Int32 n) in /home/alice/x/File.cs:line 7", "at A.B.C File.cs:line 7")]
    [InlineData("   at Microsoft.UI.Xaml.Controls.Button.OnClick()", "at Microsoft.UI.Xaml.Controls.Button.OnClick")]
    [InlineData("   at A.B.<>c__DisplayClass3_0.<M>b__0(Object o, EventArgs e)", "at A.B.<>c__DisplayClass3_0.<M>b__0")]
    public void AFrameIsAMethodAndAFileNameWithoutArgumentsOrDirectories(string raw, string expected)
    {
        Assert.Equal(expected, CrashLog.FrameLine(raw));
    }

    [Theory]
    [InlineData("--- End of stack trace from previous location ---")]
    [InlineData("")]
    [InlineData("not a frame")]
    public void ANonFrameLineIsDropped(string raw)
    {
        Assert.Null(CrashLog.FrameLine(raw));
    }

    [Fact]
    public void AnInnerExceptionIsNamedByTypeAndHResultOnly()
    {
        var outer = Thrown(() => new AggregateException("outer secret", new ArgumentException("inner secret")));
        string entry = CrashLog.Format("task", outer, Now);
        Assert.Contains("type=System.AggregateException", entry);
        Assert.Contains("inner type=System.ArgumentException", entry);
        Assert.DoesNotContain("secret", entry);
    }

    [Theory]
    [InlineData("first-run", "first-run")]
    [InlineData("xaml", "xaml")]
    [InlineData("Has Space", "invalid")]
    [InlineData("C:\\Users\\alice", "invalid")]
    [InlineData("", "invalid")]
    [InlineData("a-very-long-label-that-goes-on-and-on-beyond-the-limit", "invalid")]
    public void TheWhereIsALabelOrInvalid(string where, string shown)
    {
        Assert.Contains("where=" + shown + " ", CrashLog.Format(where, Thrower(), Now));
    }

    [Fact]
    public void ARecordIsAppendedAndTheSameTypeIsLoggedOncePerRun()
    {
        var log = new CrashLog(_dir, () => Now);
        Assert.True(log.Record("xaml", Thrower()));
        Assert.False(log.Record("xaml", Thrower()), "the same type again");
        Assert.True(log.Record("task", Thrown(() => new ArgumentException("x"))));
        string text = File.ReadAllText(Path.Combine(_dir, CrashLog.FileName));
        Assert.Equal(1, CountOf(text, "type=System.InvalidOperationException"));
        Assert.Equal(1, CountOf(text, "type=System.ArgumentException"));
    }

    [Fact]
    public void ANewRunLogsTheTypeAgain()
    {
        Assert.True(new CrashLog(_dir, () => Now).Record("xaml", Thrower()));
        Assert.True(new CrashLog(_dir, () => Now).Record("xaml", Thrower()));
        Assert.Equal(2, CountOf(File.ReadAllText(Path.Combine(_dir, CrashLog.FileName)), "type=System.InvalidOperationException"));
    }

    [Fact]
    public void TheFileIsCappedByRotatingToADotOne()
    {
        var log = new CrashLog(_dir, () => Now, capBytes: 600);
        // Distinct types, so each is logged.
        Exception[] each =
        {
            new ArgumentException(), new InvalidOperationException(), new ArgumentNullException(),
            new ArgumentOutOfRangeException(), new FormatException(), new IOException(),
            new NotSupportedException(), new TimeoutException(), new OverflowException(),
            new DivideByZeroException(), new IndexOutOfRangeException(), new ObjectDisposedException("x"),
        };
        foreach (Exception e in each)
        {
            Assert.True(log.Record("xaml", Thrown(() => e)));
        }

        string current = Path.Combine(_dir, CrashLog.FileName);
        Assert.True(new FileInfo(current).Length <= 600 + 600, "kept near the cap");
        Assert.True(File.Exists(current + ".1"));
    }

    [Fact]
    public void RecordingNeverThrowsEvenWhenTheDirectoryCannotBeWritten()
    {
        string file = Path.Combine(Path.GetTempPath(), "tc-crashlog-file-" + Guid.NewGuid().ToString("N"));
        File.WriteAllText(file, "x");
        try
        {
            // A directory path that is a file.
            Assert.False(new CrashLog(file, () => Now).Record("xaml", Thrower()));
            Assert.False(new CrashLog("", () => Now).Record("xaml", Thrower()));
        }
        finally
        {
            File.Delete(file);
        }
    }

    [Fact]
    public void ANullExceptionIsIgnored()
    {
        Assert.False(new CrashLog(_dir, () => Now).Record("xaml", null!));
    }

    [Fact]
    public void TheTrackerCountsOpenFirstRunWindowsAndNeverGoesNegative()
    {
        var tracker = new FirstRunWindowTracker();
        Assert.False(tracker.Any);
        tracker.Enter();
        tracker.Enter();
        Assert.True(tracker.Any);
        tracker.Exit();
        Assert.True(tracker.Any);
        tracker.Exit();
        Assert.False(tracker.Any);
        tracker.Exit();
        tracker.Enter();
        Assert.True(tracker.Any);
    }

    // A stack that runs through a type of this name, as a first-run fault's would.
    private static Exception WithStack(string frame, Exception? inner = null) => new StackedException(frame, inner);

    private sealed class StackedException : Exception
    {
        private readonly string _stack;

        public StackedException(string frame, Exception? inner) : base("x", inner) => _stack = "   " + frame + "()\n   at Other.Frame()";

        public override string? StackTrace => _stack;
    }

    [Theory]
    [InlineData("at TraceCommons.App.FirstRunWindow.Render")]
    [InlineData("at TraceCommons.App.FirstRunWindow.<>c.<Welcome>b__0")]
    [InlineData("at TraceCommons.Interop.FirstRun.FirstRunRunner.ContinueFromWelcome")]
    public void AFirstRunFaultIsSurvivedOnlyWhileAFirstRunWindowIsOpen(string frame)
    {
        var tracker = new FirstRunWindowTracker();
        Exception fault = WithStack(frame);
        Assert.False(tracker.Survives(fault), "no first-run window open");
        tracker.Enter();
        Assert.True(tracker.Survives(fault));
        tracker.Exit();
        Assert.False(tracker.Survives(fault), "closed again");
    }

    [Theory]
    [InlineData("at TraceCommons.App.MainWindow.Render")]
    [InlineData("at TraceCommons.App.SettingsWindow.Open")]
    [InlineData("at TraceCommons.App.AboutWindow.Show")]
    [InlineData("at TraceCommons.App.Controls.ManagedSessionsView.RenderRedTeam")]
    [InlineData("at Microsoft.UI.Xaml.Controls.Button.OnClick")]
    public void AnythingElseIsNeverSwallowedEvenWithFirstRunOpen(string frame)
    {
        var tracker = new FirstRunWindowTracker();
        tracker.Enter();
        Assert.False(tracker.Survives(WithStack(frame)));
        Assert.False(tracker.Survives(null));
        Assert.False(tracker.Survives(new InvalidOperationException()), "no stack at all");
    }

    [Fact]
    public void AFirstRunFrameInAnInnerExceptionCounts()
    {
        var tracker = new FirstRunWindowTracker();
        tracker.Enter();
        Exception outer = WithStack("at TraceCommons.App.MainWindow.Render", WithStack("at TraceCommons.App.FirstRunWindow.Render"));
        Assert.True(tracker.Survives(outer));
    }

    private static int CountOf(string text, string needle) =>
        text.Split('\n').Count(line => line.Contains(needle, StringComparison.Ordinal));
}
