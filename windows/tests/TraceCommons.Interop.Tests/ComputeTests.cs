using System;
using System.IO;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Compute section (W18-D1 (a)) against the Mac's <c>ComputeAllowance</c>,
/// <c>ComputeSnapshot</c> and <c>TCCompute</c>.
/// </summary>
/// <remarks>
/// Expected values were printed by a throwaway Swift probe compiling
/// ComputeView.swift's <c>ComputeAllowance</c> and TCShellCore/ComputeSurface.swift
/// verbatim at b8d4a6d70, linked to this worktree's FFI build, which also
/// opened a controller on an empty directory and printed its status.
/// </remarks>
public sealed class ComputeTests : IDisposable
{
    private readonly string _dir = Path.Combine(Path.GetTempPath(), "w16-compute-" + Guid.NewGuid().ToString("N"));

    public ComputeTests()
    {
        Directory.CreateDirectory(_dir);
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_dir, recursive: true);
        }
        catch (IOException)
        {
        }
    }

    [Theory]
    // probe: PARSE lines.
    [InlineData("", null)]
    [InlineData("0", null)]
    [InlineData("8", 8UL)]
    [InlineData("08", 8UL)]
    [InlineData(" 8", null)]
    [InlineData("8.0", null)]
    [InlineData("+8", 8UL)]
    [InlineData("-1", null)]
    [InlineData("-0", null)]
    [InlineData("18446744073709551615", 18446744073709551615UL)]
    [InlineData("18446744073709551616", null)]
    [InlineData("٣", null)]
    [InlineData("1e3", null)]
    public void TheAllowanceIsAWholeNumberAboveZero(string text, ulong? want)
    {
        Assert.Equal(want, ComputeAllowance.Parse(text));
    }

    [Fact]
    public void TheHandleFreeWordsAreTheCores()
    {
        // probe: COPY line.
        ComputeCopy copy = ComputeCopy.Live!;
        Assert.Equal("Compute", copy.Destination);
        Assert.Equal("Try again", copy.Retry);
        Assert.Equal("Compute settings unavailable.", copy.Unavailable);
        Assert.Equal("Enable compute", copy.Enable);
        Assert.Equal("RAM allowance (GiB)", copy.AllowanceLabel);
        Assert.Equal("Quit cancelled: compute hasn't stopped yet. Quit again once it has.", copy.QuitRefused);
    }

    [Fact]
    public void AControllerOpensReportsAndIsFreedOnceStopped()
    {
        using var compute = new TcCompute(_dir);
        // probe: SNAP line, from the same empty directory.
        ComputeSnapshot snap = ComputeSnapshot.Decode(compute.StatusJson())!;
        Assert.Equal(
            ("disabled", "worker-unavailable", "Compute is disabled", "Not available in this build."),
            (snap.State, snap.Reason, snap.Title, snap.Detail));
        Assert.False(snap.ConsentGranted);
        Assert.Null(snap.RamAllowanceGib);
        Assert.False(snap.Available || snap.CanEnable || snap.CanResume || snap.CanPause);
        Assert.False(snap.CommandPending);
        Assert.True(snap.WorkerStopped);
        // probe: CMD pause -> disabled.
        Assert.Equal("disabled", ComputeSnapshot.Decode(compute.CommandJson(new ComputeCommand.Pause()))!.State);
        Assert.True(compute.Close());
        Assert.True(compute.IsClosed);
        Assert.Equal("closed", Assert.Throws<ComputeRefusedException>(compute.StatusJson).Label);
    }

    [Fact]
    public void ASnapshotWithAnotherSchemaOrAnUnknownStateIsRefused()
    {
        using var compute = new TcCompute(_dir);
        string json = compute.StatusJson();
        Assert.NotNull(ComputeSnapshot.Decode(json));
        Assert.Null(ComputeSnapshot.Decode(json.Replace("compute_status.v1", "compute_status.v2", StringComparison.Ordinal)));
        Assert.Null(ComputeSnapshot.Decode(json.Replace("\"state\":\"disabled\"", "\"state\":\"melting\"", StringComparison.Ordinal)));
        Assert.Contains("\"state\":\"disabled\"", json);
    }

    [Fact]
    public void FreeingWaitsForStoppedEvidence()
    {
        // TCCompute.close's CloseEvidence: worker_stopped and no pending command.
        Assert.True(TcCompute.StoppedEvidence("""{"worker_stopped":true,"command_pending":false}"""));
        Assert.False(TcCompute.StoppedEvidence("""{"worker_stopped":false,"command_pending":false}"""));
        Assert.False(TcCompute.StoppedEvidence("""{"worker_stopped":true,"command_pending":true}"""));
        Assert.False(TcCompute.StoppedEvidence("""{"worker_stopped":true}"""));
        Assert.False(TcCompute.StoppedEvidence(null));
    }

    [Fact]
    public async Task TheModelOpensOnceAndSendsNoCommandTheSnapshotRefuses()
    {
        int opened = 0;
        var model = new ComputeModel(dir =>
        {
            opened++;
            return new TcCompute(dir);
        });
        await model.StartAsync(_dir);
        await model.StartAsync(_dir);
        Assert.Equal(1, opened);
        Assert.Equal("disabled", model.Snapshot!.State);
        int changes = 0;
        model.Changed += () => changes++;
        // can_enable is false: Enable sends nothing and nothing changes.
        await model.PerformAsync(new ComputeCommand.Enable(8));
        Assert.Equal(0, changes);
        Assert.False(model.Busy);
        Assert.True(model.Close());
        Assert.Null(model.Snapshot);
    }

    [Fact]
    public async Task AFailedOpenIsSaidWithTheLabelNeverASpinner()
    {
        var model = new ComputeModel(_ => throw new ComputeRefusedException("compute-open-refused"));
        await model.StartAsync(_dir);
        Assert.Null(model.Snapshot);
        Assert.Equal("compute-open-refused", model.FailureLabel);
        Assert.False(model.Busy);
    }

    /// <summary>
    /// Quitting stops the controller and frees the handle, as the Mac's
    /// quit does (<c>compute.shutdown(timeoutMilliseconds: 15_000)</c>,
    /// AppDelegate.swift:105-108), so no worker outlives the app (#2219
    /// review, defect 3). A model never opened has nothing to stop.
    /// </summary>
    [Fact]
    public async Task QuittingStopsTheControllerAndFreesTheHandle()
    {
        int opened = 0;
        var model = new ComputeModel(dir =>
        {
            opened++;
            return new TcCompute(dir);
        });
        await model.StartAsync(_dir);
        Assert.NotNull(model.Snapshot);
        Assert.True(await model.ShutdownAsync(15_000));
        Assert.Null(model.Snapshot);
        await model.StartAsync(_dir);
        Assert.Equal(1, opened);

        Assert.True(await new ComputeModel().ShutdownAsync(15_000));
    }

    [Fact]
    public void TheAppStopsComputeWhenItQuits()
    {
        string main = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.cs.txt"));
        Assert.Contains("await Controls.SettingsView.ShutdownComputeAsync();", main);
        string sections = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "SettingsView.Sections.cs.txt"));
        Assert.Contains("AppCompute.ShutdownAsync(15_000)", sections);
    }
}
