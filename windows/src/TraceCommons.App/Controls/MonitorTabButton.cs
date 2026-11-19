using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace TraceCommons.App.Controls;

/// <summary>
/// One of the Monitor window's four tabs (GlassRuleTabs,
/// MonitorWindowView.swift:972-995). A toggle button that tells UI
/// Automation it is a tab item, so a screen reader announces the strip as
/// tabs rather than as four buttons (spec E1; the W2 box dump read them as
/// Button).
/// </summary>
public sealed class MonitorTabButton : ToggleButton
{
    protected override AutomationPeer OnCreateAutomationPeer() => new MonitorTabButtonAutomationPeer(this);

    private sealed class MonitorTabButtonAutomationPeer : ToggleButtonAutomationPeer
    {
        public MonitorTabButtonAutomationPeer(MonitorTabButton owner)
            : base(owner)
        {
        }

        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.TabItem;

        protected override string GetClassNameCore() => nameof(MonitorTabButton);
    }
}
