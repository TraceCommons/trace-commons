using System;
using System.Collections.Generic;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The run-at-login entry moves from the name it used to have ("Trace
/// Commons") to the app's name, keeping the contributor's own choice.
/// </summary>
/// <remarks>
/// The value name is the entry's identity, so a bare rename would leave
/// every existing install's entry behind: the app would start twice at
/// login, and turning the toggle off would not remove the old one. And
/// Windows keeps "disabled in Startup apps" under the same name in
/// <c>StartupApproved\Run</c>, so a rename that dropped it would quietly
/// turn back on something the contributor had turned off.
/// </remarks>
public class AutostartMigrationTests
{
    private const string Exe = @"C:\Users\Ada Lovelace\AppData\Local\Gateway\TraceCommons.exe";

    /// <summary>The two keys as plain maps: the Run key's commands, StartupApproved's bytes.</summary>
    private sealed class Entries : AutostartCommand.IStartupEntries
    {
        public Dictionary<string, string> Run { get; } = new();
        public Dictionary<string, byte[]> Approved { get; } = new();

        public string? Command(string name) => Run.TryGetValue(name, out string? v) ? v : null;
        public byte[]? Approval(string name) => Approved.TryGetValue(name, out byte[]? v) ? v : null;
        public void SetCommand(string name, string command) => Run[name] = command;
        public void SetApproval(string name, byte[] approval) => Approved[name] = approval;

        public void Remove(string name)
        {
            Run.Remove(name);
            Approved.Remove(name);
        }
    }

    /// <summary>Windows' "disabled" record: 03 then a FILETIME.</summary>
    private static readonly byte[] Disabled = { 3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8 };

    [Fact]
    public void OurOldEntryMovesToTheNewNameVerbatim()
    {
        var e = new Entries();
        string stored = AutostartCommand.For(Exe);
        e.Run["Trace Commons"] = stored;

        Assert.True(AutostartCommand.Migrate(e, Exe));

        Assert.Equal(stored, e.Run[AutostartCommand.ValueName]);
        Assert.False(e.Run.ContainsKey("Trace Commons"));
        Assert.Single(e.Run);
    }

    [Fact]
    public void ADisabledEntryStaysDisabledUnderTheNewName()
    {
        var e = new Entries();
        e.Run["Trace Commons"] = AutostartCommand.For(Exe);
        e.Approved["Trace Commons"] = Disabled;

        AutostartCommand.Migrate(e, Exe);

        Assert.Equal(Disabled, e.Approved[AutostartCommand.ValueName]);
        Assert.False(e.Approved.ContainsKey("Trace Commons"));
    }

    [Fact]
    public void AnEntryWithNoApprovalRecordGetsNone()
    {
        var e = new Entries();
        e.Run["Trace Commons"] = AutostartCommand.For(Exe);

        AutostartCommand.Migrate(e, Exe);

        Assert.Empty(e.Approved);
    }

    [Fact]
    public void AnotherCopysOldEntryIsLeftAlone()
    {
        var e = new Entries();
        string other = AutostartCommand.For(@"D:\Other\TraceCommons.exe");
        e.Run["Trace Commons"] = other;
        e.Approved["Trace Commons"] = Disabled;

        Assert.False(AutostartCommand.Migrate(e, Exe));

        Assert.Equal(other, e.Run["Trace Commons"]);
        Assert.Equal(Disabled, e.Approved["Trace Commons"]);
        Assert.False(e.Run.ContainsKey(AutostartCommand.ValueName));
    }

    [Fact]
    public void AnOldEntryBesideOneAlreadyUnderTheNewNameIsRemovedNotCopied()
    {
        var e = new Entries();
        string current = AutostartCommand.For(Exe);
        e.Run["Trace Commons"] = "\"" + Exe + "\"";
        e.Run[AutostartCommand.ValueName] = current;

        Assert.True(AutostartCommand.Migrate(e, Exe));

        Assert.Equal(current, e.Run[AutostartCommand.ValueName]);
        Assert.Single(e.Run);
    }

    [Fact]
    public void TheNewNameHeldByAnotherCopyKeepsOursUnderTheOldName()
    {
        // Moving ours would overwrite the other copy's entry, or delete ours.
        var e = new Entries();
        string ours = AutostartCommand.For(Exe);
        string other = AutostartCommand.For(@"D:\Other\TraceCommons.exe");
        e.Run["Trace Commons"] = ours;
        e.Run[AutostartCommand.ValueName] = other;

        Assert.False(AutostartCommand.Migrate(e, Exe));

        Assert.Equal(ours, e.Run["Trace Commons"]);
        Assert.Equal(other, e.Run[AutostartCommand.ValueName]);
    }

    /// <summary>
    /// The contributor's own toggle claims an old-name entry wherever it
    /// points, as writing one name always did: an entry left by a moved or
    /// older copy would otherwise survive every toggle and keep starting it.
    /// </summary>
    [Fact]
    public void TheTogglesClaimAnOldEntryWhereverItPoints()
    {
        var e = new Entries();
        string stale = AutostartCommand.For(@"D:\Moved\TraceCommons.exe");
        e.Run["Trace Commons"] = stale;
        e.Approved["Trace Commons"] = Disabled;

        Assert.True(AutostartCommand.Migrate(e, Exe, claimAnyPath: true));

        Assert.False(e.Run.ContainsKey("Trace Commons"));
        Assert.Equal(stale, e.Run[AutostartCommand.ValueName]);
        Assert.Equal(Disabled, e.Approved[AutostartCommand.ValueName]);
    }

    [Fact]
    public void TheTogglesRemoveAnOldEntryBesideANewOneWhereverEitherPoints()
    {
        var e = new Entries();
        string other = AutostartCommand.For(@"D:\Other\TraceCommons.exe");
        e.Run["Trace Commons"] = AutostartCommand.For(@"D:\Moved\TraceCommons.exe");
        e.Run[AutostartCommand.ValueName] = other;

        Assert.True(AutostartCommand.Migrate(e, Exe, claimAnyPath: true));

        Assert.Equal(other, e.Run[AutostartCommand.ValueName]);
        Assert.Single(e.Run);
    }

    [Fact]
    public void NothingToMoveChangesNothing()
    {
        var e = new Entries();
        Assert.False(AutostartCommand.Migrate(e, Exe));
        Assert.Empty(e.Run);
        Assert.Empty(e.Approved);
    }

    [Fact]
    public void TheApprovalKeyIsThePerUserStartupApprovedRunKey()
    {
        Assert.Equal(
            @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
            AutostartCommand.ApprovedKeyPath);
    }
}
