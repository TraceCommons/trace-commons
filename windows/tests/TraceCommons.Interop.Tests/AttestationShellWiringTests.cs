using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of the attestation mark a contributor actually looks at.
/// </summary>
/// <remarks>
/// <c>TraceCommons.App</c> is a WinUI project and cannot be built, let alone
/// referenced, on the machines this suite runs on, so without these nothing
/// anywhere checks that the decisions in <see cref="AttestationMarkSurface"/>
/// ever reach a screen. Asserted about the sources, exactly as the
/// eligibility, routing and credential surfaces are.
/// </remarks>
public class AttestationShellWiringTests
{
    /// <summary>
    /// The row asks the shared crate for its mark rather than reading the
    /// wire field itself.
    /// </summary>
    [Fact]
    public void TheRowAsksTheSharedCrate()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/QueueEntryViewModel.cs");
        Assert.Contains("AttestationMarkSurface.Decide(", source, StringComparison.Ordinal);

        // Never the raw wire strings. A row that compared the mark itself
        // would be a second place the table lives, and it would go on
        // agreeing with an old one after the Rust changed an arm.
        foreach (string mark in new[]
        {
            "\"attested\"", "\"unattested_permanent\"", "\"unattested_configuration\"",
        })
        {
            Assert.DoesNotContain(mark, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The mark and its reason are bound, never typed, and every arm of the
    /// tone is drawn.
    /// </summary>
    [Fact]
    public void TheRowDrawsTheSharedSentences()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        foreach (string bound in new[]
        {
            "Text=\"{x:Bind AttestationText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind AttestationIsPlain, Mode=OneWay}\"",
            "Visibility=\"{x:Bind AttestationIsClear, Mode=OneWay}\"",
            "Visibility=\"{x:Bind AttestationIsAttention, Mode=OneWay}\"",
            "Text=\"{x:Bind AttestationReasonText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind HasAttestationReason, Mode=OneWay}\"",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// THE MARK IS NOT GATED ON THE ELIGIBILITY QUESTION.
    /// </summary>
    /// <remarks>
    /// The one defect this slice exists to avoid. The two sentences sit on
    /// the same row and read as siblings, which makes it natural to hang the
    /// mark off the eligibility line's own visibility. Doing so would blank
    /// the mark for exactly the contributors it was added for: an invited
    /// contributor carries no <c>eligibility</c> key, and the eligibility
    /// line is absent on every one of their rows.
    /// </remarks>
    [Fact]
    public void TheMarkIsNotGatedOnEligibility()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        foreach (Match block in Regex.Matches(
            markup, @"<TextBlock\b[^>]*Attestation[^>]*/>", RegexOptions.Singleline))
        {
            Assert.DoesNotContain("Eligibility", block.Value, StringComparison.Ordinal);
            Assert.DoesNotContain("CanContribute", block.Value, StringComparison.Ordinal);
        }

        string source = ShellSource("TraceCommons.App/ViewModels/QueueEntryViewModel.cs");
        foreach (Match member in Regex.Matches(
            source, @"public (?:bool|string) Attestation\w*[^;]*;|public (?:bool|string) HasAttestation\w*[^;]*;"))
        {
            Assert.DoesNotContain("Eligibility", member.Value, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// THE MARK DRAWS NO CONTROL.
    /// </summary>
    /// <remarks>
    /// There is no <c>tc_contribution_attestation_control</c>, deliberately.
    /// The mark describes the trace; sendability is the eligibility line's
    /// question and stays there. A button drawn off the mark would be an
    /// action invented out of a description, and pressing it would do
    /// something nobody in the shared crate decided.
    /// </remarks>
    [Fact]
    public void TheMarkDrawsNoControl()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        foreach (Match element in Regex.Matches(
            markup, @"<(?:Button|HyperlinkButton|ToggleSwitch|CheckBox)\b[^>]*>", RegexOptions.Singleline))
        {
            Assert.DoesNotContain("Attestation", element.Value, StringComparison.Ordinal);
        }

        string source = ShellSource("TraceCommons.App/ViewModels/QueueEntryViewModel.cs");
        Assert.DoesNotContain(
            "attestation_control", source, StringComparison.OrdinalIgnoreCase);
        // No declaration of one either. Matched as a declaration rather than
        // as a mention, so the prose explaining why it does not exist does
        // not read as the thing existing.
        Assert.DoesNotMatch(
            new Regex(@"extern\s+\w+\s+tc_contribution_attestation_control\b"),
            ShellSource("TraceCommons.Interop/NativeMethods.cs"));
    }

    /// <summary>
    /// The mark reaches every row, and nothing eligibility-shaped decides
    /// whether a row exists.
    /// </summary>
    [Fact]
    public void TheMarkGovernsNothingButItsOwnSentences()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        foreach (string forbidden in new[]
        {
            "ItemsSource=\"{x:Bind AttestationIsPlain",
            "ItemsSource=\"{x:Bind AttestationText",
            "IsEnabled=\"{x:Bind AttestationIsClear",
            "IsEnabled=\"{x:Bind AttestationIsPlain",
        })
        {
            Assert.DoesNotContain(forbidden, markup, StringComparison.Ordinal);
        }
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
