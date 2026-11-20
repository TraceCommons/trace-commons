using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A system setting the tray and taskbar frames draw from is read once and
/// again only when the system says it changed, never once a frame (checklist
/// O3: the review found the taskbar's light/dark registry value read on every
/// frame of a credit arrival).
/// </summary>
public class CachedReadingTests
{
    [Fact]
    public void ManyFramesReadTheSettingOnce()
    {
        int reads = 0;
        var light = new CachedReading<bool>(() =>
        {
            reads++;
            return true;
        });

        for (int frame = 0; frame < 41; frame++)
        {
            Assert.True(light.Value);
        }

        Assert.Equal(1, reads);
    }

    [Fact]
    public void AChangeIsReadAfterTheSystemSaysSo()
    {
        bool stored = false;
        var light = new CachedReading<bool>(() => stored);
        Assert.False(light.Value);

        stored = true;
        Assert.False(light.Value);
        Assert.True(light.Refresh(), "a refresh that finds a new value reports it");
        Assert.True(light.Value);
        Assert.False(light.Refresh(), "an unchanged value is not a change");
    }
}
