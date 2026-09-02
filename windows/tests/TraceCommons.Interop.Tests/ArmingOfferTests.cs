using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The arming offer's shape and words. The rule that decides *when* it
/// appears lives in the daemon (<c>ProjectPolicy::arming_suggestion</c>) and
/// is tested there; this pins what a contributor reads when it does.
/// </summary>
public class ArmingOfferTests
{
    private static JsonElement Json(string body) =>
        JsonDocument.Parse(body).RootElement;

    [Fact]
    public void ParsesTheDaemonsShape()
    {
        ArmingOffer? offer = ArmingOffer.Parse(Json(
            """{"project_id":"proj_ab12","project_label":"api","contributed_count":5}"""));
        Assert.NotNull(offer);
        Assert.Equal("proj_ab12", offer!.ProjectId);
        Assert.Equal("api", offer.ProjectLabel);
        Assert.Equal(5, offer.ContributedCount);
    }

    /// <summary>
    /// The daemon answers with an empty object when it has nothing to
    /// suggest. That must draw no card, not a card about nothing.
    /// </summary>
    [Fact]
    public void AnEmptyAnswerIsNoOffer()
    {
        Assert.Null(ArmingOffer.Parse(Json("{}")));
    }

    [Fact]
    public void AnOfferWithoutAnIdIsNoOffer()
    {
        Assert.Null(ArmingOffer.Parse(Json("""{"project_label":"api","contributed_count":9}""")));
        Assert.Null(ArmingOffer.Parse(Json("""{"project_id":""}""")));
    }

    /// <summary>
    /// The evidence is stated before the question, so a contributor who reads
    /// only the first line still learns why they are being asked.
    /// </summary>
    [Fact]
    public void EvidenceNamesTheProjectAndTheCount()
    {
        Assert.Equal(
            "You've contributed from api 5 times.",
            ArmingOfferCopy.Evidence("api", 5));
    }

    /// <summary>
    /// The daemon's threshold is five, so this branch is unreachable today.
    /// It is here because the sentence must be right about whatever count it
    /// is handed, and "contributed from api 1 times" is not.
    /// </summary>
    [Fact]
    public void EvidenceIsSingularForOne()
    {
        Assert.Equal(
            "You've contributed from api once.",
            ArmingOfferCopy.Evidence("api", 1));
    }

    [Fact]
    public void TheQuestionNamesTheProject()
    {
        Assert.Equal(
            "Contribute from api automatically?",
            ArmingOfferCopy.Question("api"));
    }

    [Fact]
    public void TheButtonsCarryTheirActions()
    {
        Assert.Equal("Turn on automatic contributing", ArmingOfferCopy.Confirm);
        Assert.Equal("Not now", ArmingOfferCopy.Decline);
    }

    /// <summary>
    /// "Not now", not "No": the daemon silences the offer for thirty days
    /// rather than forever, and the button must not promise otherwise.
    /// </summary>
    [Fact]
    public void DecliningDoesNotSoundPermanent()
    {
        string lower = ArmingOfferCopy.Decline.ToLowerInvariant();
        Assert.DoesNotContain("never", lower, StringComparison.Ordinal);
        Assert.DoesNotContain("don't ask", lower, StringComparison.Ordinal);
    }
}
