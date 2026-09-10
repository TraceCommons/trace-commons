using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of the balance surface a contributor actually looks at.
///
/// <para>
/// <c>TraceCommons.App</c> is a WinUI project and cannot be referenced from
/// this suite, so without these nothing checks that the decisions in
/// <see cref="NearAiBalanceSurface"/> ever reach a screen -- which is exactly
/// the state this surface was in before: the daemon read the balance, the
/// copy and the C ABI existed, and no shell drew a figure anywhere. Asserted
/// about the sources, the way the routing, credential and eligibility
/// surfaces are.
/// </para>
/// </summary>
public class BalanceShellWiringTests
{
    /// <summary>
    /// Every sentence and every figure is ASKED FOR rather than assembled.
    /// </summary>
    /// <remarks>
    /// The formatting is the part worth pinning. A view model that divided by
    /// its own constant, or wrote its own "Left to spend", would be a second
    /// place the money rules live and would go on agreeing with itself after
    /// the Rust changed one.
    /// </remarks>
    [Fact]
    public void ThePageAsksTheSharedTableForEveryLine()
    {
        string source = Uncommented(ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs"));

        foreach (string call in new[]
        {
            "NearAiBalanceSurface.StateLine(_balance, _copy)",
            "NearAiBalanceSurface.Tone(_balance)",
            "NearAiBalanceSurface.Action(_balance)",
            "NearAiBalanceSurface.Amount(_balance.RemainingNanos, _balance.Scale)",
            "NearAiBalanceSurface.RemainingLine(_balance.RemainingNanos, _balance.Scale)",
            "NearAiBalanceSurface.LimitLine(_balance.SpendLimitNanos, _balance.Scale)",
            "NearAiBalanceSurface.SpentLine(_balance.TotalSpentNanos, _balance.Scale)",
            "NearAiBalanceSurface.ObservedLine(_balance.ObservedAt",
            "NearAiBalanceSurface.Parse(",
        })
        {
            Assert.Contains(call, source, StringComparison.Ordinal);
        }

        // None of the money rules is reproduced here: no scale constant, no
        // dollar sign, and no state string compared by hand.
        foreach (string forbidden in new[]
        {
            "1_000_000_000", "1000000000", "\"$", "$0.00", "ToString(\"C",
            "\"known\"", "\"no_session\"", "\"session_expired\"",
            "\"no_organization\"", "\"unavailable\"",
        })
        {
            Assert.DoesNotContain(forbidden, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// AN ABSENT FIGURE IS DRAWN AS NO FIGURE. The amount's own emptiness is
    /// what decides whether a figure appears, and the sentence that explains
    /// an account with no ceiling takes its place when it does not.
    /// </summary>
    /// <remarks>
    /// The one branch this page owns, and it is a branch on "is there a
    /// figure" rather than on any value: the ABI answers the empty string for
    /// an absent amount and never for a zero, so a real $0.00 balance takes
    /// the figure arm exactly as $8.50 does.
    /// </remarks>
    [Fact]
    public void TheFigureAndItsAbsenceAreDrawnFromTheAmountsOwnEmptiness()
    {
        string source = Uncommented(ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs"));

        Assert.Matches(
            new Regex(@"public bool HasBalanceAmount =>\s*BalanceAmountText\.Length > 0;"),
            source);
        Assert.Matches(
            new Regex(
                @"public bool HasBalanceRemainingText =>\s*"
                + @"!HasBalanceAmount && BalanceRemainingText\.Length > 0;"),
            source);

        // Nothing decides it by comparing a rendered figure to a zero.
        Assert.DoesNotContain(
            "BalanceAmountText ==", source, StringComparison.Ordinal);
        Assert.DoesNotContain(
            "RemainingNanos == 0", source, StringComparison.Ordinal);
    }

    /// <summary>
    /// The card draws the title, the figure, all four sentences and every
    /// tone arm.
    /// </summary>
    /// <remarks>
    /// Every arm, because a tone with no arm is a state whose sentence never
    /// appears -- and on this row three of the five states are the ones a
    /// contributor most needs to read.
    /// </remarks>
    [Fact]
    public void TheCardDrawsTheFigureAndEveryToneArm()
    {
        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");

        foreach (string bound in new[]
        {
            "Text=\"{x:Bind ViewModel.BalanceTitle, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.BalanceWhat, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.BalanceAmountText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasBalanceAmount, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.BalanceRemainingText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasBalanceRemainingText, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.BalanceLimitText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasBalanceLimitText, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.BalanceSpentText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasBalanceSpentText, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.BalanceObservedText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasBalanceObservedText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.BalanceIsNeutral, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.BalanceIsHeld, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.BalanceIsClear, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.BalanceIsAttention, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.BalanceIsRefused, Mode=OneWay}\"",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }

        // The state sentence is drawn once per tone and nowhere else, so an
        // empty "known" sentence cannot leave a blank line above the figure.
        Assert.Equal(
            5,
            Regex.Matches(markup, @"Text=""\{x:Bind ViewModel\.BalanceStateText, Mode=OneWay\}""").Count);
        Assert.Contains(
            "Visibility=\"{x:Bind ViewModel.HasBalanceStateText, Mode=OneWay}\"",
            markup,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// NOTHING PAINTS A LOW FIGURE RED. The figure carries no tone of its
    /// own.
    /// </summary>
    /// <remarks>
    /// A threshold nobody set, on an account whose ceiling may not exist, is
    /// the one judgement this surface forbids -- and the tone that does exist
    /// means the read succeeded, not that the money is comfortable. So the
    /// amount's own TextBlock has no Foreground at all, and no tone flag is
    /// bound to anything but a sentence.
    /// </remarks>
    [Fact]
    public void TheFigureIsNeverPaintedFromItsValue()
    {
        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");

        Match figure = Regex.Match(
            markup,
            @"<TextBlock[^>]*Text=""\{x:Bind ViewModel\.BalanceAmountText, Mode=OneWay\}""[^>]*/>",
            RegexOptions.Singleline);
        Assert.True(figure.Success, "the balance figure is not drawn");
        Assert.DoesNotContain("Foreground", figure.Value, StringComparison.Ordinal);

        // And no view model property turns an amount into a colour.
        string source = Uncommented(ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs"));
        foreach (string forbidden in new[]
        {
            "BalanceIsLow", "BalanceIsEmpty", "BalanceIsHealthy", "BalanceWarning",
        })
        {
            Assert.DoesNotContain(forbidden, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The one action is the SIGN-IN ROW'S, reached through the sign-in row's
    /// own call.
    /// </summary>
    /// <remarks>
    /// Not a parallel ceremony. The words, the consequence sentence and the
    /// press all go through the credential surface, so a balance that says
    /// the session expired opens the same browser flow the card above it
    /// offers -- and the branch deciding whether to offer it at all is the
    /// ABI's, never this page's.
    /// </remarks>
    [Fact]
    public void TheBalanceActionIsTheSignInRowsOwn()
    {
        string source = Uncommented(ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs"));

        Assert.Contains(
            "NearAiCredentialSurface.ActionLabel(BalanceOfferedAction, _copy)",
            source,
            StringComparison.Ordinal);
        Assert.Contains(
            "NearAiCredentialSurface.ActionPreamble(BalanceOfferedAction, _copy)",
            source,
            StringComparison.Ordinal);
        Assert.Matches(
            new Regex(
                @"public Task<NearAiCredentialAttempt\?> PressBalanceAsync\(NearAiSignInProvider\? provider = null\)"
                + @"(?:(?!\n    \}).)*StartCredentialAsync\(selected\)",
                RegexOptions.Singleline),
            source);

        // The offer is the ABI's answer, and it is withheld while the sign-in
        // row is already offering one: two Obtain buttons on one screen are
        // two ways to mint the same key.
        Assert.Matches(
            new Regex(
                @"public bool HasBalanceAction =>\s*"
                + @"BalanceOfferedAction != CredentialAction\.None\s*"
                + @"&& !HasCredentialAction\s*&& _copy is not null;"),
            source);

        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");
        Assert.Contains(
            "Visibility=\"{x:Bind ViewModel.HasBalanceAction, Mode=OneWay}\"",
            markup,
            StringComparison.Ordinal);
        Assert.Contains("Click=\"OnBalanceAction\"", markup, StringComparison.Ordinal);
    }

    /// <summary>
    /// The balance is read, and re-read whenever the sign-in changes.
    /// </summary>
    /// <remarks>
    /// A figure left over from before a sign-in was forgotten is a claim
    /// about an account this machine can no longer reach. The read follows
    /// the credential read for that reason, and no snapshot pushes one:
    /// nothing in settings carries a balance.
    /// </remarks>
    [Fact]
    public void TheBalanceIsReadAfterEverySignInRead()
    {
        string source = Uncommented(ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs"));

        Assert.Contains(
            "DaemonProtocol.Methods.NearAiBalance", source, StringComparison.Ordinal);
        Assert.Matches(
            new Regex(
                @"public async Task LoadCredentialAsync\(\)"
                + @"(?:(?!\n    /// <summary>).)*await LoadBalanceAsync\(\)",
                RegexOptions.Singleline),
            source);

        // A failed read says the daemon did not report one. It never keeps a
        // stale figure and never claims the account is empty.
        Assert.Matches(
            new Regex(
                @"_balance = (?:response\.IsError \|\| response\.Result is null\s*\?\s*)?"
                + @"NearAiBalance\.Unreported"),
            source);
    }

    private static string Uncommented(string source) =>
        string.Join(
            "\n",
            source
                .Split('\n')
                .Where(line =>
                {
                    string trimmed = line.TrimStart();
                    return !trimmed.StartsWith("//", StringComparison.Ordinal)
                        && !trimmed.StartsWith("///", StringComparison.Ordinal);
                }));

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
