using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Which of the generated theme dictionaries (GlassTokens.xaml, BrandTokens.xaml,
/// DesignSystem.xaml) a brush is read from: WinUI names the dark one
/// <c>Default</c>, and a high contrast theme wins over either appearance,
/// as the system's own colours replace the app's. Code that reads a brush
/// itself must pick by the window's theme now, not the one the app started
/// in: a box run switching dark to light while the app ran left the graph's
/// pills and the rail's card in their dark colours (2026-11-19).
/// </summary>
public class ThemeDictionaryKeyTests
{
    [Theory]
    [InlineData(false, true, "Light")]
    [InlineData(false, false, "Default")]
    [InlineData(true, true, "HighContrast")]
    [InlineData(true, false, "HighContrast")]
    public void TheDictionaryFollowsTheThemeWithHighContrastFirst(bool highContrast, bool light, string key) =>
        Assert.Equal(key, ThemeDictionaryKey.For(highContrast, light));
}
