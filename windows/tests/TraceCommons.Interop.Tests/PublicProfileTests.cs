using System;
using System.Text;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// What this shell is allowed to say about a public handle, checked on a
/// machine that cannot build WinUI at all.
///
/// This is the reason <see cref="PublicProfileCopy"/> lives in the interop
/// assembly rather than in a view model. The Linux shell holds the identical
/// assertions in <c>crates/trace-commons-contributor-gtk/src/copy.rs</c> and
/// macOS holds them in <c>PublicProfileCopy.swift</c>'s render-time check.
/// The three shells must not diverge, and these tests are what stops this one
/// drifting.
/// </summary>
public sealed class PublicProfileCopyTests
{
    /// <summary>
    /// The invariant, pinned as an invariant rather than as a string.
    /// </summary>
    /// <remarks>
    /// <c>handle_persisted: false</c> is a failed LOCAL CACHE WRITE, not a
    /// failed claim: by the time the flag exists the server has already taken
    /// the handle. Both sentences must therefore open by saying the
    /// contributor is on the roster, and neither may carry the vocabulary of
    /// a refusal. Asserted as properties so that rewording the copy stays
    /// legal and reversing its meaning does not.
    ///
    /// This is the Windows counterpart of
    /// <c>a_profile_that_was_published_never_reads_as_one_that_was_not</c>.
    /// </remarks>
    [Fact]
    public void AProfileThatWasPublishedNeverReadsAsOneThatWasNot()
    {
        foreach (string sentence in new[]
                 {
                     PublicProfileCopy.Published,
                     PublicProfileCopy.PublishedNotCached,
                 })
        {
            Assert.StartsWith("You're on the roster", sentence, StringComparison.Ordinal);

            string lower = sentence.ToLowerInvariant();
            foreach (string forbidden in new[]
                     {
                         "couldn't publish",
                         "failed",
                         "wasn't published",
                         "nothing changed",
                     })
            {
                Assert.DoesNotContain(forbidden, lower, StringComparison.Ordinal);
            }
        }

        // And the uncached one still says the weaker true thing, rather than
        // being the same sentence twice.
        Assert.NotEqual(PublicProfileCopy.Published, PublicProfileCopy.PublishedNotCached);
        Assert.Contains(
            "until you save it once more",
            PublicProfileCopy.PublishedNotCached,
            StringComparison.Ordinal);
        Assert.Contains(
            "That doesn't change anything about what is public.",
            PublicProfileCopy.PublishedNotCached,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// The flag routes to those two sentences and to nothing else. This is
    /// the branch that would silently invert if someone read
    /// <c>handle_persisted</c> as "did the claim work".
    /// </summary>
    [Fact]
    public void TheUncachedBranchIsTheOnlyThingHandlePersistedChanges()
    {
        Assert.Equal(PublicProfileCopy.Published, PublicProfileCopy.PublishedSentence(true));
        Assert.Equal(
            PublicProfileCopy.PublishedNotCached,
            PublicProfileCopy.PublishedSentence(false));
    }

    /// <summary>
    /// The mirror rule. The row is gone from the server whether or not the
    /// local clear stuck, so neither sentence may leave a contributor
    /// thinking they are still listed.
    /// </summary>
    [Fact]
    public void AWithdrawalThatHappenedNeverReadsAsOneThatDidNot()
    {
        foreach (string sentence in new[]
                 {
                     PublicProfileCopy.LeftRoster,
                     PublicProfileCopy.LeftRosterNotCached,
                 })
        {
            Assert.StartsWith("You've left the roster", sentence, StringComparison.Ordinal);
            Assert.Contains("isn't published any more", sentence, StringComparison.Ordinal);
        }

        Assert.Equal(PublicProfileCopy.LeftRoster, PublicProfileCopy.LeftRosterSentence(true));
        Assert.Equal(
            PublicProfileCopy.LeftRosterNotCached,
            PublicProfileCopy.LeftRosterSentence(false));
    }

    /// <summary>
    /// A refusal happens before or instead of the <c>PUT</c>, so in every one
    /// of these cases the handle did not go up -- and the contributor has to
    /// be able to tell this apart from the published-but-uncached case.
    /// </summary>
    [Fact]
    public void EveryRefusalSaysNothingWasPublished()
    {
        foreach (string label in new[]
                 {
                     "handle-required",
                     "handle-too-short",
                     "handle-too-long",
                     "handle-invalid-character",
                     "handle-invalid-boundary",
                     "handle-consecutive-separators",
                     "handle-reserved",
                     "bio-too-long",
                     "bio-invalid-character",
                     "bio-required-or-null",
                     "bio-invalid",
                     "not-logged-in",
                     "profile-update-failed",
                     "daemon-not-running",
                 })
        {
            string sentence = PublicProfileCopy.FailureSentence(label);

            Assert.Contains(
                "Nothing was published and nothing changed.",
                sentence,
                StringComparison.Ordinal);
            Assert.DoesNotContain("on the roster", sentence, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// A failed withdrawal says the opposite of a failed claim, because the
    /// opposite is what is true: the handle is still up.
    /// </summary>
    [Fact]
    public void AFailedWithdrawalSaysTheHandleIsStillPublished()
    {
        foreach (string label in new[] { "not-logged-in", "profile-withdraw-failed" })
        {
            string sentence = PublicProfileCopy.LeaveFailureSentence(label);

            Assert.Contains("still on the roster", sentence, StringComparison.Ordinal);
            Assert.DoesNotContain("Nothing was published", sentence, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// A label this build has never heard of gets the generic sentence, and
    /// never the label itself.
    /// </summary>
    /// <remarks>
    /// The daemon does not forward the underlying error -- it can carry a
    /// server response body or a URL -- but this is the one seam where such a
    /// string could reach a screen, so the fallback is pinned rather than
    /// assumed.
    /// </remarks>
    [Fact]
    public void AnUnknownLabelIsNeverEchoed()
    {
        const string Invented = "https://ingest.example.invalid/v1/community/profile 500";

        string claim = PublicProfileCopy.FailureSentence(Invented);
        Assert.DoesNotContain(Invented, claim, StringComparison.Ordinal);
        Assert.StartsWith("The request didn't go through.", claim, StringComparison.Ordinal);

        string leave = PublicProfileCopy.LeaveFailureSentence(Invented);
        Assert.DoesNotContain(Invented, leave, StringComparison.Ordinal);

        // Null is the same case: a response frame with no message at all.
        Assert.StartsWith(
            "The request didn't go through.",
            PublicProfileCopy.FailureSentence(null),
            StringComparison.Ordinal);
    }

    /// <summary>
    /// The copy is the Linux shell's, word for word.
    /// </summary>
    /// <remarks>
    /// Compared WHOLE against <c>copy.rs</c> rather than by keyword, for the
    /// same reason <see cref="WithdrawCopyTests"/> compares the canonical
    /// withdrawal bodies whole: a paraphrase that kept every keyword would
    /// still be a paraphrase, and three shells wording one consent act three
    /// ways is three different promises about what becomes public.
    /// </remarks>
    [Fact]
    public void TheSentencesAreStillTheLinuxShellsOwnWords()
    {
        Assert.Equal(
            "Attribution only -- being listed grants no data use at all. Leaving the roster "
            + "removes you from future snapshots.",
            PublicProfileCopy.Footnote);

        Assert.Equal(
            "I understand my handle and aggregate counts become public. Leaving the roster "
            + "removes me from future snapshots.",
            PublicProfileCopy.GoPublicAcknowledgement);

        Assert.Equal(
            "Nothing is pre-checked, and Go public stays off until the acknowledgement is on. "
            + "This changes attribution only -- it grants no data use.",
            PublicProfileCopy.GoPublicFootnote);

        Assert.Equal(
            "Your handle -- real handles only, no pseudonyms. Aggregate counts: accepted, "
            + "novelty credit, accept rate. The date you went public. Your bio, if you write one.",
            PublicProfileCopy.PublishedBody);

        Assert.Equal(
            "Your traces or anything in them. Per-trace data of any kind. Anything about "
            + "sessions you didn't send.",
            PublicProfileCopy.NeverBody);

        Assert.Equal("On the roster since March 4, 2026", PublicProfileCopy.OnRosterSince("March 4, 2026"));
    }

    /// <summary>
    /// The bio counter counts UTF-8 bytes, which is the unit the budget is
    /// denominated in.
    /// </summary>
    /// <remarks>
    /// Characters would let this window report a bio as comfortably inside a
    /// budget the server then refuses -- the counter lying about the only
    /// thing it exists to report.
    /// </remarks>
    [Fact]
    public void TheBioCounterCountsBytesNotCharacters()
    {
        Assert.Equal("0/280", PublicProfileCopy.BioCounter(string.Empty));
        Assert.Equal("0/280", PublicProfileCopy.BioCounter(null));
        Assert.Equal("5/280", PublicProfileCopy.BioCounter("plain"));

        // Four characters, twelve bytes.
        const string Emoji = "🌱🌱🌱";
        Assert.Equal(12, Encoding.UTF8.GetByteCount(Emoji));
        Assert.Equal("12/280", PublicProfileCopy.BioCounter(Emoji));
    }
}

/// <summary>
/// Reading the daemon's profile answer, and building the one request that
/// changes it.
/// </summary>
public sealed class PublicProfileProtocolTests
{
    private static PublicProfileResult Parse(string json) =>
        PublicProfileResult.Parse(JsonDocument.Parse(json).RootElement)!;

    /// <summary>
    /// <c>on_roster</c> is the verdict, not the presence of a handle.
    /// </summary>
    /// <remarks>
    /// The field exists to answer exactly this question, and a shell that
    /// answered it some other way would be a second opinion about who is
    /// public.
    /// </remarks>
    [Fact]
    public void OnRosterDecidesWhoIsListed()
    {
        Assert.Equal("ada", Parse("""{"on_roster":true,"handle":"ada"}""").ListedHandle);

        // A stale handle beside on_roster:false is not a listing.
        Assert.Null(Parse("""{"on_roster":false,"handle":"ada"}""").ListedHandle);
        Assert.Null(Parse("""{"on_roster":true}""").ListedHandle);
    }

    /// <summary>
    /// An absent <c>handle_persisted</c> reads as true, matching the Linux
    /// shell: the only thing the false branch adds is a warning about this
    /// window's own cache, and a daemon that said nothing has not earned it.
    /// </summary>
    [Fact]
    public void AnAbsentPersistedFlagIsNotAWarning()
    {
        Assert.True(Parse("""{"on_roster":true,"handle":"ada"}""").CachedLocally);
        Assert.True(Parse("""{"on_roster":true,"handle_persisted":true}""").CachedLocally);
        Assert.False(Parse("""{"on_roster":true,"handle_persisted":false}""").CachedLocally);
    }

    /// <summary>
    /// The whole path, end to end: the daemon's answer to a claim it could
    /// not cache still reaches the contributor as a published profile.
    /// </summary>
    /// <remarks>
    /// The two halves are each pinned above; this is the join between them,
    /// which is where a shell that read <c>handle_persisted</c> as "did it
    /// work" would actually go wrong.
    /// </remarks>
    [Fact]
    public void AClaimTheDaemonCouldNotCacheIsStillReportedAsPublished()
    {
        PublicProfileResult result = Parse(
            """{"on_roster":true,"handle":"ada","bio":null,"handle_persisted":false}""");

        string sentence = PublicProfileCopy.PublishedSentence(result.CachedLocally);

        Assert.StartsWith("You're on the roster", sentence, StringComparison.Ordinal);
        Assert.Equal("ada", result.ListedHandle);
    }

    [Fact]
    public void AnEmptyBioAndAnAbsentOneAreTheSameThing()
    {
        Assert.Equal(string.Empty, Parse("""{"on_roster":true,"handle":"ada"}""").PublishedBio);
        Assert.Equal(
            string.Empty,
            Parse("""{"on_roster":true,"handle":"ada","bio":null}""").PublishedBio);
        Assert.Equal(
            "builds things",
            Parse("""{"on_roster":true,"handle":"ada","bio":"builds things"}""").PublishedBio);
    }

    [Fact]
    public void AnUnparsableAnswerIsNullRatherThanAThrow()
    {
        Assert.Null(PublicProfileResult.Parse(null));
    }

    /// <summary>
    /// A <c>public_since</c> this build cannot parse produces no line at all,
    /// rather than a wrong one.
    /// </summary>
    [Fact]
    public void AnUnreadableRosterDateDrawsNothing()
    {
        Assert.Null(Parse("""{"on_roster":true,"handle":"ada"}""").OnRosterSinceLine());
        Assert.Null(
            Parse("""{"on_roster":true,"public_since":"the fourth of March"}""")
                .OnRosterSinceLine());

        string? line = Parse("""{"on_roster":true,"public_since":"2026-03-04T12:00:00Z"}""")
            .OnRosterSinceLine();
        Assert.NotNull(line);
        Assert.StartsWith("On the roster since ", line, StringComparison.Ordinal);
    }

    /// <summary>
    /// The bio key is always on the wire, because the <c>PUT</c> replaces the
    /// whole profile and the daemon refuses an omitted <c>bio</c> rather than
    /// guessing which of "no bio" and "leave it alone" was meant.
    /// </summary>
    [Fact]
    public void AnEmptyBioBoxIsSentAsNullAndNeverOmitted()
    {
        using JsonDocument empty = JsonDocument.Parse(
            PublicProfileRequest.Serialize("ada", "   "));

        Assert.True(empty.RootElement.TryGetProperty("bio", out JsonElement bio));
        Assert.Equal(JsonValueKind.Null, bio.ValueKind);
        Assert.Equal("ada", empty.RootElement.GetProperty("handle").GetString());

        using JsonDocument written = JsonDocument.Parse(
            PublicProfileRequest.Serialize("  ada  ", "  builds things  "));

        Assert.Equal("ada", written.RootElement.GetProperty("handle").GetString());
        Assert.Equal("builds things", written.RootElement.GetProperty("bio").GetString());
    }

    /// <summary>
    /// Nothing in this shell pre-judges a handle.
    /// </summary>
    /// <remarks>
    /// The daemon and the server share one copy of the handle rules; a second
    /// copy here is how a handle this window accepts becomes one the server
    /// refuses. Equally, the claim is NOT gated on the local consent-scope
    /// list: the server authorizes the <c>PUT</c> against the grant ceiling,
    /// the local set can be narrower than what the credential carries, and
    /// refusing here would refuse contributors the server would have allowed.
    /// So a handle this shell believes to be malformed still goes out and
    /// still comes back as the daemon's own verdict.
    /// </remarks>
    [Fact]
    public void AHandleThisShellDislikesIsStillSent()
    {
        using JsonDocument sent = JsonDocument.Parse(
            PublicProfileRequest.Serialize("--not a handle--", null));

        Assert.Equal("--not a handle--", sent.RootElement.GetProperty("handle").GetString());
    }
}
