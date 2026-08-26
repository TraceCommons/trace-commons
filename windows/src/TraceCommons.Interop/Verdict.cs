using System;

namespace TraceCommons.Interop;

/// <summary>
/// The contributor's own verdict on how a session went: the optional
/// <c>outcome</c> parameter of <c>approve</c>.
/// </summary>
/// <remarks>
/// Three accepted values and no others. Absent means the contributor did not
/// answer -- the daemon records <c>TaskSuccess::Unknown</c> and the approval
/// proceeds normally, so no verdict is not an error and must never gate the
/// approve control. Anything else is refused as <c>bad_params</c> with the
/// label <c>outcome-invalid</c>, and that refusal approves NOTHING: a shell
/// that sends <c>null</c> or an empty string in place of "no answer" turns
/// every unanswered submit into a failed one.
///
/// See <c>docs/contributor-daemon-ipc-v1_1.md</c>, "The <c>outcome</c>
/// verdict". The wire values here are the same three
/// <c>trace-commons-contributor-gtk</c> sends; the labels a contributor
/// reads live in <see cref="VerdictCopy"/>.
/// </remarks>
public static class Verdict
{
    /// <summary>The session did what was asked.</summary>
    public const string Worked = "worked";

    /// <summary>It did some of it.</summary>
    public const string Partly = "partly";

    /// <summary>It did not.</summary>
    public const string Failed = "failed";

    /// <summary>
    /// Whether <paramref name="outcome"/> is one of the three the daemon
    /// accepts. <c>null</c> is not: absence is expressed by omitting the
    /// parameter, which is <see cref="IsAbsent"/>, not by a value.
    /// </summary>
    public static bool IsRecognised(string? outcome) =>
        outcome is Worked or Partly or Failed;

    /// <summary>
    /// Whether this is "the contributor did not answer" rather than a
    /// verdict. Only <c>null</c> is: an empty or blank string is a value the
    /// daemon would refuse, and treating it as absence here is what keeps it
    /// from reaching the wire.
    /// </summary>
    public static bool IsAbsent(string? outcome) => outcome is null;

    /// <summary>
    /// Returns <paramref name="outcome"/> unchanged if the daemon would
    /// accept it, and throws otherwise.
    /// </summary>
    /// <remarks>
    /// Refused here rather than over the socket on purpose. An unrecognised
    /// value approves nothing, so shipping it would present a contributor
    /// with a submit that silently did not happen; this turns the same
    /// mistake into a test failure instead.
    /// </remarks>
    public static string Require(string outcome)
    {
        if (!IsRecognised(outcome))
        {
            throw new ArgumentException(
                "outcome must be worked, partly or failed; omit it entirely for no answer.",
                nameof(outcome));
        }

        return outcome;
    }
}
