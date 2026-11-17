using System;
using System.IO;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Quitting asks only when it would stop something live, and then in one
/// sentence from the core (<c>quit_copy.rs</c>, exported as
/// <c>tc_quit_reason_prompt_json</c>).
/// </summary>
public sealed class QuitPromptTests
{
    private const string Valid =
        "{\"title\":\"T\",\"body\":\"B.\",\"confirm\":\"Quit\",\"cancel\":\"Cancel\"}";

    [Fact]
    public void NothingLiveQuitsWithoutAskingTheCore()
    {
        bool asked = false;
        QuitDecision decision = QuitPrompt.Decide(false, false, out QuitPrompt? prompt, (_, _) =>
        {
            asked = true;
            return Valid;
        });

        Assert.Equal(QuitDecision.QuitNow, decision);
        Assert.Null(prompt);
        Assert.False(asked);
    }

    [Fact]
    public void AnUnreadablePromptWhileSomethingIsLiveRefuses()
    {
        Assert.Equal(QuitDecision.Refuse, QuitPrompt.Decide(false, true, out _, (_, _) => null));
        Assert.Equal(QuitDecision.Refuse, QuitPrompt.Decide(true, false, out _, (_, _) => "{}"));
    }

    [Theory]
    [InlineData(true, false, "Quitting stops compute on this computer.")]
    [InlineData(false, true, "Quitting stops Private AI on this computer.")]
    [InlineData(true, true, "Quitting stops compute and Private AI on this computer.")]
    public void SomethingLiveAsksInOneSentenceFromTheCore(bool compute, bool privateAI, string body)
    {
        QuitDecision decision = QuitPrompt.Decide(compute, privateAI, out QuitPrompt? prompt);

        Assert.Equal(QuitDecision.Ask, decision);
        Assert.Equal(body, prompt!.Body);
        Assert.Equal("Quit Near AI Gateway?", prompt.Title);
        Assert.Equal("Quit", prompt.Confirm);
        Assert.Equal("Cancel", prompt.Cancel);
    }

    [Fact]
    public void TheMainWindowAsksTheCoreRatherThanTypingTheSentence()
    {
        string source = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.cs.txt"));

        Assert.DoesNotContain("Quitting stops", source, StringComparison.Ordinal);
        Assert.DoesNotContain("\"Quit Near AI Gateway?\"", source, StringComparison.Ordinal);
        Assert.Contains("QuitPrompt.Decide(", source, StringComparison.Ordinal);
    }
}
