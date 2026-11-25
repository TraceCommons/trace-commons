using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Model calls as a top-level destination: the rail item, the page, the tray
/// entry and the two rules they all hang on.
///
/// <para>
/// The first rule is that only a clear tone may be painted as working. A rail
/// badge and a tray glyph both invite a green dot, and painting a refusal as
/// "on" is the fail-open this whole surface exists to prevent.
/// </para>
/// <para>
/// The second is that the indicator derives from the tone and never from the
/// settings boolean. The switch says what was ASKED FOR; the indicator says
/// what is true, and the two disagree exactly when it matters -- the switch
/// on, the listener refused to start.
/// </para>
/// </summary>
public class PrivateInferenceDestinationTests
{
    private static PrivateInferenceCopy Copy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static PrivateInferenceState State(string label, ushort? port = null) =>
        new(label, port);

    /// <summary>
    /// One predicate, and only one value satisfies it. Every indicator in
    /// this shell asks this rather than deciding for itself.
    /// </summary>
    [Fact]
    public void OnlyTheClearToneReadsAsWorking()
    {
        Assert.True(PrivateInferenceTone.Clear.ReadsAsWorking());
        foreach (PrivateInferenceTone tone in new[]
        {
            PrivateInferenceTone.Neutral,
            PrivateInferenceTone.Held,
            PrivateInferenceTone.Attention,
            PrivateInferenceTone.Refused,
        })
        {
            Assert.False(tone.ReadsAsWorking(), $"{tone} must not be painted as working");
        }
    }

    /// <summary>
    /// A value from a later daemon reaches the predicate as an int, and it
    /// must not arrive as the working light.
    /// </summary>
    [Fact]
    public void AnUnrecognisedAbiValueDoesNotReadAsWorking()
    {
        foreach (int stranger in new[]
        {
            0, 1, 2, 3, 10, 14, 20, 25, 26, 99, -1, int.MaxValue, int.MinValue,
        })
        {
            Assert.False(
                PrivateInferenceSurface.FromAbiTone(stranger).ReadsAsWorking(),
                $"the unrecognised ABI value {stranger} must not be painted as working");
        }
    }

    /// <summary>
    /// Every state the daemon reports, through the whole chain a rail badge
    /// and a tray glyph actually take. Only the running listener with
    /// somewhere to send lights up.
    /// </summary>
    [Fact]
    public void OnlyARunningListenerLightsTheIndicator()
    {
        Assert.True(PrivateInferenceSurface.Tone(State("running")).ReadsAsWorking());
        foreach (string label in new[]
        {
            "", "off", "stopping", "running_no_backends", "running_elsewhere",
            "port_in_use", "start_failed", "crashed", "a_state_from_a_later_daemon",
        })
        {
            Assert.False(
                PrivateInferenceSurface.Tone(State(label)).ReadsAsWorking(),
                $"{label} must not read as working");
        }
    }

    /// <summary>
    /// The tray entry is computed from the reported state, never from the
    /// switch. The two are handed in separately here precisely so this can be
    /// asserted: switch on, listener refused, indicator dark.
    /// </summary>
    [Fact]
    public void TheTrayIndicatorFollowsTheStateAndNotTheSwitch()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string refused in new[] { "port_in_use", "start_failed", "crashed" })
        {
            PrivateInferenceTrayEntry entry =
                PrivateInferenceTrayEntry.For(copy, State(refused), on: true);
            Assert.True(entry.Available);
            Assert.True(entry.On);
            Assert.False(
                entry.ReadsAsWorking,
                $"the tray painted {refused} as working because the switch was on");
        }

        PrivateInferenceTrayEntry running =
            PrivateInferenceTrayEntry.For(copy, State("running", 8463), on: true);
        Assert.True(running.ReadsAsWorking);

        // And the mirror image: the switch off while a listener this app
        // started is still coming down. Nothing claims to be working.
        Assert.False(
            PrivateInferenceTrayEntry.For(copy, State("stopping"), on: false).ReadsAsWorking);
    }

    /// <summary>
    /// Every word on the tray entry is the payload's. None is spelled here.
    /// </summary>
    [Fact]
    public void TheTrayEntryTakesEveryWordFromThePayload()
    {
        PrivateInferenceCopy copy = Copy();
        PrivateInferenceTrayEntry entry =
            PrivateInferenceTrayEntry.For(copy, State("running", 8463), on: true);

        Assert.Equal(copy.Destination, entry.Label);
        Assert.Equal(copy.TrayTurnOff, entry.ActionText);
        Assert.Equal(
            PrivateInferenceSurface.StateLine(State("running", 8463), copy), entry.StateText);
    }

    /// <summary>
    /// The tray may turn it OFF and may not turn it ON.
    /// </summary>
    /// <remarks>
    /// Turning it off only ever reduces what this computer will answer, so it
    /// is safe from a menu with nothing on screen. Turning it on changes what
    /// anything else running here may send through, charged to the
    /// contributor's accounts, and the sentence saying so
    /// (<c>offer_exposure</c>) is the reason this became a destination rather
    /// than a switch. A menu press that enabled it would route around that
    /// sentence, so while it is off the entry navigates and writes nothing.
    /// Asserted over every state the daemon can report, including the ones
    /// where the switch and the listener disagree.
    /// </remarks>
    [Fact]
    public void TheTrayTurnsItOffAndOpensTheScreenToTurnItOn()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string label in new[]
        {
            "", "off", "running", "stopping", "running_no_backends", "running_elsewhere",
            "port_in_use", "start_failed", "crashed", "a_state_from_a_later_daemon",
        })
        {
            PrivateInferenceTrayEntry off =
                PrivateInferenceTrayEntry.For(copy, State(label), on: false);
            Assert.Equal(PrivateInferenceTrayAction.OpenDestination, off.Action);
            Assert.False(
                off.ActionWrites,
                $"the tray offered to turn model calls on from the menu at {label}");
            Assert.Equal(copy.TrayOpenToTurnOn, off.ActionText);

            PrivateInferenceTrayEntry on =
                PrivateInferenceTrayEntry.For(copy, State(label), on: true);
            Assert.Equal(PrivateInferenceTrayAction.StopAnswering, on.Action);
            Assert.True(on.ActionWrites, $"the tray could not turn model calls off at {label}");
            Assert.Equal(copy.TrayTurnOff, on.ActionText);
        }
    }

    /// <summary>
    /// The action is a function of the switch alone, never of the tone: a
    /// listener that refused to start still leaves something to turn off.
    /// </summary>
    [Fact]
    public void TheTrayActionDoesNotFollowTheIndicator()
    {
        PrivateInferenceCopy copy = Copy();
        PrivateInferenceTrayEntry refused =
            PrivateInferenceTrayEntry.For(copy, State("port_in_use"), on: true);
        Assert.False(refused.ReadsAsWorking);
        Assert.True(refused.ActionWrites);
        Assert.Equal(PrivateInferenceTrayAction.StopAnswering, refused.Action);
    }

    /// <summary>
    /// No row in this menu carries a check mark any more. The model-calls row
    /// is an action, not a switch: a checked row invites a press to uncheck
    /// it, which on this surface is the press that must not exist.
    /// </summary>
    [Fact]
    public void TheModelCallsRowIsAnActionAndNotACheckedSwitch()
    {
        string tray = ShellSource("TraceCommons.App/TrayIcon.cs");
        Assert.DoesNotContain("MF_CHECKED", tray, StringComparison.Ordinal);
        Assert.DoesNotContain("MF_UNCHECKED", tray, StringComparison.Ordinal);
        Assert.Contains("ActionText", tray, StringComparison.Ordinal);
        Assert.Contains("ActionWrites", tray, StringComparison.Ordinal);
        Assert.Contains("PrivateInferenceStopRequested", tray, StringComparison.Ordinal);
        Assert.DoesNotContain("PrivateInferenceToggleRequested", tray, StringComparison.Ordinal);
    }

    /// <summary>
    /// The window's handler for the tray's one write turns it off. It does
    /// not flip whatever it finds, because the tray is not allowed to be the
    /// thing that turned it on.
    /// </summary>
    [Fact]
    public void TheWindowsTrayHandlerTurnsItOffRatherThanFlippingIt()
    {
        string window = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains("_tray.PrivateInferenceStopRequested +=", window, StringComparison.Ordinal);
        Assert.Contains("TurnOffAsync()", window, StringComparison.Ordinal);

        string body = MethodBody(window, "private void OnTrayPrivateInferenceStopRequested()");
        Assert.Contains("TurnOffAsync", body, StringComparison.Ordinal);
        Assert.DoesNotContain("ToggleAsync", body, StringComparison.Ordinal);

        string page = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml.cs");
        Assert.Contains("TurnOffAsync() => ViewModel.SetAsync(false)", page, StringComparison.Ordinal);
    }

    /// <summary>
    /// A method body, from its signature to the first line that closes a
    /// brace at the method's own indent. Crude on purpose; see
    /// <see cref="Property"/>.
    /// </summary>
    private static string MethodBody(string source, string signature)
    {
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{signature} is gone");
        int end = source.IndexOf("\n    }\n", start, StringComparison.Ordinal);
        Assert.True(end > start, $"{signature} does not close");
        return source[start..end];
    }

    /// <summary>
    /// No payload, no entry. The same stance the settings card takes: a menu
    /// row with a switch and no sentence beside it is the shape that says
    /// "on" over a listener that refused to start.
    /// </summary>
    [Fact]
    public void AMissingPayloadLeavesTheTrayEntryOut()
    {
        PrivateInferenceTrayEntry entry =
            PrivateInferenceTrayEntry.For(null, State("running"), on: true);
        Assert.False(entry.Available);
        Assert.False(entry.ReadsAsWorking);
        Assert.Equal(string.Empty, entry.Label);
        Assert.Equal(string.Empty, entry.StateText);
        Assert.Equal(string.Empty, entry.ActionText);
        Assert.False(entry.ActionWrites);
    }

    /// <summary>
    /// The rail label is short enough to sit in a 184px rail beside the other
    /// three, and it is the Rust's word rather than this shell's.
    /// </summary>
    [Fact]
    public void TheRailLabelArrivesFromTheRust()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.False(string.IsNullOrWhiteSpace(copy.Destination));
        Assert.False(string.IsNullOrWhiteSpace(copy.Subtitle));
        Assert.True(
            copy.Destination.Length <= 24,
            $"the rail label does not fit beside the other three: {copy.Destination}");
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }

    /// <summary>
    /// The rail item's label is bound, never typed. The label is the one word
    /// a contributor navigates by, and a shell that spelled it itself would
    /// go on spelling the old one after a rename in the Rust.
    /// </summary>
    [Fact]
    public void TheRailItemBindsItsLabelRatherThanSpellingIt()
    {
        // The rail is the Monitor window's tab strip now (W2): the Private AI
        // tab's title is the core's tab word, as on the Mac
        // (MonitorWindowView.swift:39-47), set from code, and the markup
        // spells no label for it.
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        Assert.Contains("x:Name=\"TabInferenceLabel\" Grid.Column=\"0\" VerticalAlignment=\"Center\" />", markup, StringComparison.Ordinal);
        string shell = ShellSource("TraceCommons.App/MainWindow.Shell.cs");
        Assert.Contains("string title = MonitorWords.TabTitle(tab);", shell, StringComparison.Ordinal);
        Assert.Matches(@"case MonitorTab\.Inference:\s*ViewModel\.ShowPrivateInference\(\);\s*_ = Observed\.Run\(PreparePrivateInferenceTabAsync\(\)", shell);
        Assert.Matches(@"case MonitorTab\.Traces:\s*ViewModel\.ShowQueue\(\);\s*_ = Observed\.Run\(StartContributionsCoreAsync\(\)", shell);
    }

    /// <summary>
    /// Not one surface in this shell spells the destination's name.
    ///
    /// The Rust owns it, and the reason it is that word rather than the
    /// setting's internal one is that turning this on does not make a call
    /// private: it moves where the call is answered, and the call still goes
    /// on to whoever answers it. A shell that typed either phrase would be
    /// making a promise the Rust deliberately does not.
    /// </summary>
    [Fact]
    public void NoShellSourceSpellsTheDestinationOrCallsItPrivate()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source");
        Assert.True(Directory.Exists(root), $"the shell sources were not copied to {root}");

        var offenders = new List<string>();
        foreach (string file in Directory.EnumerateFiles(root, "*.txt", SearchOption.AllDirectories))
        {
            string relative = Path.GetRelativePath(root, file).Replace('\\', '/');
            string source = File.ReadAllText(file);

            // Comments are prose about this surface and quote it on purpose.
            string uncommented = string.Join(
                "\n",
                source.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n')
                    .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
            uncommented = Regex.Replace(
                uncommented, "<!--.*?-->", string.Empty, RegexOptions.Singleline);

            foreach (string forbidden in new[]
            {
                "Model calls", "Private inference", "private inference",
            })
            {
                if (uncommented.Contains(forbidden, StringComparison.Ordinal))
                {
                    offenders.Add($"{relative} spells \"{forbidden}\"");
                }
            }
        }

        Assert.True(
            offenders.Count == 0,
            "The destination's name comes from private_inference_copy.rs across the ABI.\n"
            + string.Join("\n", offenders));
    }

    /// <summary>
    /// The rail badge, the page's own status line and the tray glyph all
    /// derive from the tone. None of them reads the switch.
    /// </summary>
    [Fact]
    public void NoIndicatorInThisShellFollowsTheSwitch()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs");
        Assert.Contains(
            "PrivateInferenceSurface.Tone(_privateInferenceState)",
            viewModel,
            StringComparison.Ordinal);
        Assert.Contains("ReadsAsWorking()", viewModel, StringComparison.Ordinal);

        // The rail badge's own property, and what it is allowed to read.
        string badge = Property(viewModel, "public bool PrivateInferenceIsWorking");
        Assert.DoesNotContain("_privateInferenceOn", badge, StringComparison.Ordinal);
        Assert.DoesNotContain("PrivateInferenceOn", badge, StringComparison.Ordinal);

        // The tab's dot (W2): the reported state through the core's menu
        // table, never the switch; the window shows it from that property.
        string dot = Property(viewModel, "public DotStatus PrivateInferenceDot");
        Assert.Contains("_privateInferenceState", dot, StringComparison.Ordinal);
        Assert.DoesNotContain("PrivateInferenceOn", dot, StringComparison.Ordinal);
        Assert.Contains(
            "ViewModel.PrivateInferenceDot", ShellSource("TraceCommons.App/MainWindow.Shell.cs"), StringComparison.Ordinal);

        // And the markup never binds the switch.
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        Assert.DoesNotContain(
            "x:Bind ViewModel.PrivateInferenceOn", markup, StringComparison.Ordinal);
    }

    /// <summary>
    /// The destination page draws the shared sentences and picks its colour
    /// from the tone, exactly as the settings card does.
    /// </summary>
    [Fact]
    public void TheDestinationPageIsDrawnFromTheSharedSentences()
    {
        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml")
            + ShellSource("TraceCommons.App/Controls/PrivateInferenceSettingsView.xaml");

        foreach (string bound in new[]
        {
            "ViewModel.Heading.Title",
            "ViewModel.Heading.Subtitle",
            "ViewModel.Exposure",
            "ViewModel.ToggleText",
            "ViewModel.StateText",
            "ViewModel.AppliesAtOnce",
            "ViewModel.StateIsRefused",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }

        // Every rendered string on the page is a binding, never a literal.
        foreach (Match match in Regex.Matches(markup, "(Text|Content|Header)=\"([^\"]*)\""))
        {
            Assert.StartsWith("{x:Bind", match.Groups[2].Value, StringComparison.Ordinal);
        }

        string viewModel = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");
        foreach (string sourced in new[]
        {
            "_copy?.SettingsTitle",
            "_copy?.Subtitle",
            "_copy?.OfferExposure",
            "_copy?.SettingsToggle",
            "_copy?.SettingsAppliesAtOnce",
        })
        {
            Assert.Contains(sourced, viewModel, StringComparison.Ordinal);
        }

        Assert.Contains(
            "PrivateInferenceSurface.Tone(_state)", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("StateText ==", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("StateText.Contains", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("StateText.StartsWith", viewModel, StringComparison.Ordinal);
    }

    /// <summary>
    /// Not one string literal in the destination's view model. A paraphrase
    /// would go in as a fallback beside a payload read -- friendlier, wrong,
    /// and passing every behavioural test in this file.
    /// </summary>
    [Fact]
    public void NoWordingIsAuthoredInTheDestinationViewModel()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");
        string uncommented = string.Join(
            "\n",
            source.Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            Assert.Fail(
                $"{match.Value} is a string literal in the destination's view model. Every "
                + "sentence on this surface comes from private_inference_copy.rs across the ABI.");
        }
    }

    /// <summary>
    /// The four in-app shortcuts, and no global hotkey.
    ///
    /// A system-wide hotkey is a different thing entirely -- it steals a key
    /// combination from every other application on the machine -- and it was
    /// cut deliberately. RegisterHotKey appearing anywhere in this shell is
    /// that cut being quietly undone.
    /// </summary>
    [Fact]
    public void TheShortcutsAreInAppAndThereIsNoGlobalHotkey()
    {
        // The keyboard map is registered from code on the window's own tree
        // (W2-D5 (a)): Ctrl+1..4 by position, Ctrl+comma, Ctrl+Shift+M.
        string shell = ShellSource("TraceCommons.App/MainWindow.Shell.cs");
        foreach (string key in new[] { "Number1", "Number2", "Number3", "Number4" })
        {
            Assert.Contains($"VirtualKey.{key}", shell, StringComparison.Ordinal);
        }

        Assert.Contains("WindowRoot.KeyboardAccelerators.Add(", shell, StringComparison.Ordinal);
        Assert.Contains("privateAI.Invoked += OnTogglePrivateInferenceAccelerator;", shell, StringComparison.Ordinal);

        // Comments are stripped first: the prose beside the accelerators
        // names the API it is deliberately not using.
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source");
        foreach (string file in Directory.EnumerateFiles(root, "*.txt", SearchOption.AllDirectories))
        {
            string source = File.ReadAllText(file).Replace("\r\n", "\n", StringComparison.Ordinal);
            string uncommented = string.Join(
                "\n",
                source.Split('\n')
                    .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
            uncommented = Regex.Replace(
                uncommented, "<!--.*?-->", string.Empty, RegexOptions.Singleline);

            Assert.DoesNotContain("RegisterHotKey", uncommented, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The tray offers the destination and the switch, and it asks the shared
    /// model for both rather than composing them itself.
    /// </summary>
    [Fact]
    public void TheTrayEntryIsBuiltFromTheSharedModel()
    {
        string tray = ShellSource("TraceCommons.App/TrayIcon.cs");
        Assert.Contains("PrivateInferenceTrayEntry", tray, StringComparison.Ordinal);
        Assert.Contains("ReadsAsWorking", tray, StringComparison.Ordinal);
        Assert.Contains("PrivateInferenceStopRequested", tray, StringComparison.Ordinal);
        Assert.Contains("PrivateInferenceRequested", tray, StringComparison.Ordinal);

        string window = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains(
            "_tray.PrivateInferenceStopRequested +=", window, StringComparison.Ordinal);
        Assert.Contains("_tray.PrivateInferenceRequested +=", window, StringComparison.Ordinal);
    }

    [Fact]
    public void PrivateAiControlsAreInSettingsAndTheTabLinksToThem()
    {
        string tab = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");
        Assert.DoesNotContain("ViewModel.ToggleText", tab, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.CredentialActionText", tab, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.HarnessesTitle", tab, StringComparison.Ordinal);
        Assert.Contains("OnSettingsLink", tab, StringComparison.Ordinal);

        string settings = ShellSource("TraceCommons.App/Controls/PrivateInferenceSettingsView.xaml");
        Assert.Contains("ViewModel.ToggleText", settings, StringComparison.Ordinal);
        Assert.Contains("ViewModel.CredentialActionText", settings, StringComparison.Ordinal);
        Assert.Contains("ViewModel.HarnessesTitle", settings, StringComparison.Ordinal);
    }

    /// <summary>
    /// The body of one property, from its signature to the line that closes
    /// it. Crude on purpose, matching <c>PrivateInferenceTests.MethodBody</c>:
    /// this suite cannot compile the C# it is reading, and a parser here would
    /// be a second thing to get wrong.
    /// </summary>
    private static string Property(string source, string signature)
    {
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{signature} is gone");
        int end = source.IndexOf(";\n", start, StringComparison.Ordinal);
        Assert.True(end > start, $"{signature} does not close");
        return source[start..end];
    }
    /// <summary>
    /// The keyboard chord may stop answering and may never start it.
    /// </summary>
    /// <remarks>
    /// Asserted on the shared rule and on the handler's source together,
    /// because the rule alone proves nothing about what the chord does with
    /// it: the previous handler called ToggleAsync, which inverts the switch
    /// and writes straight through, so a single keypress could open a
    /// listener to everything on this machine with the exposure sentence
    /// unread.
    /// </remarks>
    [Fact]
    public void TheKeyboardChordCanStopAnsweringAndCanNeverStartIt()
    {
        Assert.Equal(
            PrivateInferenceTrayAction.StopAnswering,
            PrivateInferenceTrayEntry.ActionFor(on: true));
        Assert.Equal(
            PrivateInferenceTrayAction.OpenDestination,
            PrivateInferenceTrayEntry.ActionFor(on: false));

        string handler = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        int at = handler.IndexOf("OnTogglePrivateInferenceAccelerator", StringComparison.Ordinal);
        Assert.True(at >= 0, "the accelerator handler is in this shell");
        string body = handler[at..];
        int end = body.IndexOf("\n    private ", StringComparison.Ordinal);
        if (end > 0)
        {
            body = body[..end];
        }

        Assert.DoesNotContain("ToggleAsync", body);
        Assert.Contains("ActionFor", body);
        Assert.Contains("TurnOffAsync", body);
    }

}
