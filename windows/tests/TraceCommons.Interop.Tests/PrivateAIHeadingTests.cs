using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Private AI tab's heading row against the Mac's <c>statusRow</c> and
/// <c>subtitle</c> (InferenceViews.swift:474-492), through the real core
/// tables (private_inference_copy.rs <c>menu_state</c>).
/// </summary>
public sealed class PrivateAIHeadingTests
{
    private static PrivateInferenceCopy Copy() => PrivateInferenceSurface.Copy()!;

    [Fact]
    public void RunningIsTheOnWordOnGreenWithTheCreditSentence()
    {
        PrivateInferenceCopy copy = Copy();
        PrivateAIHeading row = PrivateAIHeading.For(new PrivateInferenceState("running", 4141), copy);
        Assert.Equal(copy.InspectorStatus, row.Title);
        Assert.Equal("On", row.Word);
        Assert.Equal(DotStatus.On, row.Dot);
        Assert.Equal(copy.Subtitle + " " + copy.SubtitleCredit, row.Subtitle);
        Assert.Equal(PrivateInferenceSurface.StateLine(new PrivateInferenceState("running", 4141), copy), row.Sentence);
        Assert.NotEqual(row.Word, row.Sentence);
    }

    [Fact]
    public void RunningWithNoBackendsIsOnInAmberWithoutTheCreditSentence()
    {
        PrivateInferenceCopy copy = Copy();
        PrivateAIHeading row = PrivateAIHeading.For(new PrivateInferenceState("running_no_backends", 4141), copy);
        Assert.Equal("On", row.Word);
        Assert.Equal(DotStatus.Ask, row.Dot);
        Assert.Equal(copy.Subtitle, row.Subtitle);
    }

    [Fact]
    public void APortInUseIsRedAndOffIsGrey()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(DotStatus.Outside, PrivateAIHeading.For(new PrivateInferenceState("port_in_use", 4141), copy).Dot);
        Assert.Equal(DotStatus.Off, PrivateAIHeading.For(new PrivateInferenceState("off", null), copy).Dot);
    }

    // A core that gave no word draws the sentence, on grey.
    [Fact]
    public void NoMenuWordDrawsTheSentenceOnGrey()
    {
        PrivateInferenceCopy copy = Copy();
        PrivateAIHeading row = PrivateAIHeading.For("running", copy, "the sentence", string.Empty, PrivateInferenceTone.Clear);
        Assert.Equal(("the sentence", DotStatus.Off, "the sentence"), (row.Word, row.Dot, row.Sentence));
    }
}
