using System;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// Where keyboard focus goes when a step is built again in place after a
/// commit or a failure (checklist O2, copy guide rule 13): back to the
/// control the person was on, by its stable key, or to the step's heading
/// when that control is gone or cannot take focus now. The key the person
/// chose outlives a rebuild that disabled its control (a commit running),
/// so focus returns there once it is enabled again.
/// </summary>
public sealed class FirstRunFocus
{
    /// <summary>The step heading's key.</summary>
    public const string Heading = "heading";

    /// <summary>The last control the person put focus on in this step, or null.</summary>
    public string? Wanted { get; private set; }

    /// <summary>Focus landed on a keyed control; the heading taking it is not the person's choice.</summary>
    public void Focused(string? key)
    {
        if (!string.IsNullOrEmpty(key) && key != Heading)
        {
            Wanted = key;
        }
    }

    /// <summary>A new step: nothing on it was chosen yet.</summary>
    public void StepChanged() => Wanted = null;

    /// <summary>The key to focus after a rebuild: the wanted control if it can take focus, else the heading.</summary>
    public string Target(Func<string, bool> canFocus) =>
        Wanted is { } key && canFocus(key) ? key : Heading;
}
