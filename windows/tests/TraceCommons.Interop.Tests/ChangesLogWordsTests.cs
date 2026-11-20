using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The change log's words against the Mac's <c>SettingsContent</c>
/// (SettingsView.swift:18, 23-28, 111) and the core's table
/// (shell_words_copy.rs:542-557; preview_copy.rs:1711). The rows were
/// shell-written on Windows before ("Permissions changed", "Nothing has
/// been changed.").
/// </summary>
public class ChangesLogWordsTests
{
    [Theory]
    [InlineData("armed-auto-upload", "repo-a", "Automatic turned on for repo-a")]
    [InlineData("consent-scopes-changed", null, "Data uses changed")]
    [InlineData("queue-bulk-approved", "", "Whole queue approved")]
    [InlineData("a-label-this-build-does-not-know", "repo-b", "Changed repo-b")]
    public void EachChangeIsTheCoresSentenceThenTheProject(string action, string? project, string want)
    {
        Assert.Equal(want, SettingsLegacyWords.AuditSentence(action, project));
    }

    [Fact]
    public void TheHeadingAndTheEmptyLineAreTheCores()
    {
        Assert.Equal("Change log", SettingsLegacyWords.ChangesHeading);
        Assert.Equal("No changes yet.", SettingsLegacyWords.NothingChanged);
    }

    [Fact]
    public void NoTableMeansNoSentence()
    {
        Assert.Equal(string.Empty, SettingsLegacyWords.AuditSentence(null, "armed-auto-upload", "x"));
    }
}
