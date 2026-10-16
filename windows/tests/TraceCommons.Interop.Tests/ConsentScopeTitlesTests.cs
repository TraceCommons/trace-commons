using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The preview sheet's Permissions tab names every scope the upload asks
/// for. A scope the core has no title for, or a bundle that could not be
/// read, shows the wire name, as GTK does: a held scope is never hidden.
/// </summary>
public class ConsentScopeTitlesTests
{
    [Fact]
    public void AScopeWithoutATitleStillYieldsARow()
    {
        var titles = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["debugging_evaluation"] = "Finding bugs and measuring agents",
        };

        IReadOnlyList<string> rows = ConsentScopeTitles.PermissionTitles(
            new[] { "debugging_evaluation", "future_scope" }, titles);

        Assert.Equal(new[] { "Finding bugs and measuring agents", "future_scope" }, rows);
    }

    [Fact]
    public void AnUnreadableBundleYieldsOneRowPerScope()
    {
        IReadOnlyList<string> rows = ConsentScopeTitles.PermissionTitles(
            new[] { "debugging_evaluation", "model_training" },
            new Dictionary<string, string>(StringComparer.Ordinal));

        Assert.Equal(new[] { "debugging_evaluation", "model_training" }, rows);
    }

    [Fact]
    public void EveryScopeTheCoreTitlesIsNamedByIt()
    {
        string[] scopes =
        {
            "debugging_evaluation", "benchmark_only", "ranking_training", "model_training", "public_attribution",
        };

        IReadOnlyList<string> rows = ConsentScopeTitles.PermissionTitles(scopes);

        Assert.Equal(scopes.Length, rows.Count);
        for (int i = 0; i < scopes.Length; i++)
        {
            Assert.Equal(ConsentScopeTitles.Title(scopes[i]), rows[i]);
            Assert.NotEqual(scopes[i], rows[i]);
        }
    }
}
