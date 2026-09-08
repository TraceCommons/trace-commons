using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of this surface a contributor actually looks at.
///
/// <para>
/// <c>TraceCommons.App</c> is a WinUI project and cannot be built, let alone
/// referenced, on the macOS and Linux machines this suite runs on -- so
/// without these, nothing anywhere checks that the decisions in
/// <see cref="ContributionEligibilitySurface"/> ever reach a screen. Asserted
/// about the sources, the same way the routing and credential surfaces are:
/// a view that re-derived a send control from a state string would pass every
/// behavioural test in <see cref="ContributionEligibilityTests"/>.
/// </para>
/// </summary>
public class EligibilityShellWiringTests
{
    /// <summary>
    /// The row asks the shared crate for its eligibility rather than reading
    /// the wire field itself.
    /// </summary>
    [Fact]
    public void TheRowAsksTheSharedCrate()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/QueueEntryViewModel.cs");
        Assert.Contains("ContributionEligibilitySurface.Decide(", source, StringComparison.Ordinal);

        // Never the raw wire strings. A row that compared the state itself
        // would be a fourth place the branch table lives, and it would go on
        // agreeing with an old one after the Rust changed an arm.
        foreach (string state in new[]
        {
            "\"eligible\"", "\"ineligible_permanent\"", "\"ineligible_configuration\"",
        })
        {
            Assert.DoesNotContain(state, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// EVERY SESSION IS SHOWN. The queue draws no eligibility condition on
    /// anything that decides whether a row exists.
    /// </summary>
    /// <remarks>
    /// The rule this whole surface turns on. Hiding a contributor's own work
    /// is its own dishonesty and makes this app look as though it had not
    /// noticed files they know it can see, so the only thing eligibility may
    /// govern is the send control -- checked by name below.
    /// </remarks>
    [Fact]
    public void EligibilityGatesTheSendControlAndNothingElse()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");

        // The one place CanContribute is allowed to appear: the Submit
        // button's own visibility.
        Assert.Single(Regex.Matches(markup, @"\{x:Bind CanContribute[^}]*\}"));
        Assert.Contains(
            "Visibility=\"{x:Bind CanContribute, Mode=OneWay}\"",
            markup,
            StringComparison.Ordinal);

        // Nothing eligibility-derived reaches the ItemsSource, the repeater or
        // any row-level Visibility other than the three sentence arms.
        foreach (string forbidden in new[]
        {
            "ItemsSource=\"{x:Bind CanContribute",
            "ItemsSource=\"{x:Bind HasEligibilityText",
            "ItemsSource=\"{x:Bind EligibilityIsPlain",
        })
        {
            Assert.DoesNotContain(forbidden, markup, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The sentence and its reason are bound, never typed, and every arm of
    /// the tone is drawn.
    /// </summary>
    /// <remarks>
    /// All three arms, because a row that carries no send control has to say
    /// why. A tone with no arm would leave that row silent, which is the one
    /// outcome forbidden here -- see
    /// <see cref="QueueEntryViewModel"/>'s <c>EligibilityIsPlain</c>, which is
    /// the complement of the other two rather than a fourth named tone.
    /// </remarks>
    [Fact]
    public void TheRowDrawsTheSharedSentences()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        foreach (string bound in new[]
        {
            "Text=\"{x:Bind EligibilityText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind EligibilityIsPlain, Mode=OneWay}\"",
            "Visibility=\"{x:Bind EligibilityIsClear, Mode=OneWay}\"",
            "Visibility=\"{x:Bind EligibilityIsAttention, Mode=OneWay}\"",
            "Text=\"{x:Bind EligibilityReasonText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind HasEligibilityReason, Mode=OneWay}\"",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The preview sheet's Contribute is held by the same rule.
    /// </summary>
    /// <remarks>
    /// "Look inside" is a second route to the send. A button the row withheld
    /// would otherwise be waiting one click behind it, which is the same
    /// defect with an extra step in front of it.
    /// </remarks>
    [Fact]
    public void TheSheetsContributeIsHeldByTheSameRule()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs");
        Assert.Matches(
            new Regex(@"public bool CanContribute =>[^;]*Entry\.CanContribute", RegexOptions.Singleline),
            source);

        string markup = ShellSource("TraceCommons.App/Controls/PreviewSheet.xaml");
        foreach (string bound in new[]
        {
            "Text=\"{x:Bind ViewModel.EligibilityText, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.EligibilityReasonText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasEligibilityText, Mode=OneWay}\"",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
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
