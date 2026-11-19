using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace TraceCommons.App.Look;

/// <summary>
/// The visual state manager of the re-templated controls (spec W5-D3 (a)):
/// it runs a template's transitions, which carry the Mac's curves and
/// durations, only while things may move; otherwise a control goes to its
/// new state at once, which is its final frame (GlassMotion.swift: nil under
/// Reduce Motion). Set on a template's root as
/// <c>VisualStateManager.CustomVisualStateManager</c>.
/// </summary>
public sealed class GlassVisualStateManager : VisualStateManager
{
    protected override bool GoToStateCore(
        Control control, FrameworkElement templateRoot, string stateName, VisualStateGroup group, VisualState state, bool useTransitions) =>
        base.GoToStateCore(control, templateRoot, stateName, group, state, useTransitions && GlassMotionSettings.MayMove);
}
