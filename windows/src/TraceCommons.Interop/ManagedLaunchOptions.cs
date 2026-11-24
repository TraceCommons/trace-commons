using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

public enum ManagedModelMode { None, Catalogue, Text }

public sealed record ManagedLaunchOptions(
    string? Model, bool CollectTraces, string CliOptions,
    bool KeepSavedModel = false, string? BackgroundModel = null)
{
    public static ManagedModelMode ModelMode(string tool, string connection) => tool == "opencode"
        ? connection == "near_ai_signin" ? ManagedModelMode.Catalogue : ManagedModelMode.Text
        : connection is "near_ai" or "near_ai_signin" ? ManagedModelMode.Catalogue : ManagedModelMode.None;

    public static bool OffersBackground(string tool, string connection) =>
        tool != "codex" && ModelMode(tool, connection) == ManagedModelMode.Catalogue;

    internal void Apply(Dictionary<string, object?> request, ManagedAccount account, ManagedCapabilities capabilities)
    {
        var mode = ModelMode(account.Tool, account.Connection);
        request["model"] = mode != ManagedModelMode.None && !string.IsNullOrWhiteSpace(Model) ? Model.Trim() : null;
        request["collect_traces"] = CollectTraces;
        request["cli_options"] = CliOptions;
        if (mode != ManagedModelMode.None && capabilities.KeepSavedModel && KeepSavedModel)
            request["keep_saved_model"] = true;
        if (capabilities.BackgroundModel && OffersBackground(account.Tool, account.Connection) && BackgroundModel is not null)
            request["background_model"] = BackgroundModel;
    }
}

public sealed record ManagedFolderPreferences(
    string Tool, string? Model, bool Remembered,
    [property: JsonPropertyName("collect_traces")] bool CollectTraces,
    [property: JsonPropertyName("cli_options")] string CliOptions,
    [property: JsonPropertyName("background_model")] string? BackgroundModel,
    [property: JsonPropertyName("project_workflows")] bool ProjectWorkflows);

public sealed record ManagedCatalogueModel(string Id, string? Name, string? Tier)
{
    public string Label => string.IsNullOrWhiteSpace(Name) ? Id : Name.Trim();
}

public sealed record ManagedBackgroundChoice([property: JsonPropertyName("default")] string Default, int Set);

public sealed record ManagedLaunchCatalogue(
    ManagedCatalogueModel[] Models,
    [property: JsonPropertyName("default")] string? Default,
    string[]? Favorites,
    [property: JsonPropertyName("tier_order")] string[]? TierOrder,
    Dictionary<string, ManagedBackgroundChoice>? Background,
    [property: JsonPropertyName("background_sets")] string[][]? BackgroundSets,
    [property: JsonPropertyName("background_models")] ManagedCatalogueModel[]? BackgroundModels,
    [property: JsonPropertyName("unavailable_favorites")] string[]? UnavailableFavorites = null)
{
    private static readonly JsonSerializerOptions Wire = new() { PropertyNameCaseInsensitive = true };

    public static ManagedLaunchCatalogue? Parse(JsonElement value) => value.Deserialize<ManagedLaunchCatalogue>(Wire);

    public string ModelDisplayName(string id) => Models.FirstOrDefault(model => model.Id == id)?.Label ?? id;

    public IEnumerable<ManagedCatalogueModel> OrderedModels()
    {
        var tiers = TierOrder ?? [];
        var favorites = (Favorites ?? []).ToHashSet(StringComparer.Ordinal);
        return Models.OrderBy(model => favorites.Contains(model.Id) ? -1 : TierRank(model.Tier));

        int TierRank(string? tier)
        {
            int rank = tier is null ? -1 : Array.IndexOf(tiers, tier);
            return rank >= 0 ? rank : tiers.Length;
        }
    }

    public static IEnumerable<(string Id, bool Unavailable)> FavoriteRows(
        string[] stored, ManagedLaunchCatalogue? catalogue)
    {
        if (catalogue is null) return stored.Select(id => (id, false));
        var unavailable = (catalogue.UnavailableFavorites ?? []).ToHashSet(StringComparer.Ordinal);
        var order = catalogue.Models.Select(model => model.Id).ToArray();
        return stored.Where(id => !unavailable.Contains(id))
            .OrderBy(id => Array.IndexOf(order, id) is >= 0 and var rank ? rank : int.MaxValue)
            .ThenBy(id => id, StringComparer.Ordinal)
            .Select(id => (id, false))
            .Concat(stored.Where(unavailable.Contains).Select(id => (id, true)));
    }
}
