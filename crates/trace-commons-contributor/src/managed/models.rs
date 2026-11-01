//! Which NEAR AI model a managed session runs on.
//!
//! The session's proxy pins the NEAR AI backend only, so a tool may ask
//! for any model NEAR AI lists and is served that model. Each tool still
//! starts on one default, and it must be chosen on purpose: NEAR AI lists
//! its catalogue alphabetically, and the first entry is neither a sensible
//! default for every tool nor always a chat model. Every model id a tool
//! can send must be a listed chat model, or IronWire gives the request a
//! substitute of the same tier: [`claude_env`] maps Claude Code's model
//! aliases, `super::codex` lists the models for Codex, and OpenCode's
//! config lists them for OpenCode.
//!
//! The rule for the default:
//!
//! 1. Chat models only. NEAR AI describes each entry: an entry is kept when
//!    its `input_modalities` include `text` and its `output_modalities` are
//!    exactly `["text"]`, which drops image, audio, embedding and decision
//!    models. Two kinds still answer text in, text out without being chat
//!    models, a reranker and the privacy filter, so ids naming one of
//!    [`EXCLUDED`] are dropped too. That list also covers an entry that
//!    carries no modality fields at all. `supported_features` is not used:
//!    it is incomplete (current GPT models list no `tools`).
//! 2. A model the person chose must be one of those, or nothing starts.
//! 3. Otherwise, Claude Code and OpenCode start in the most private tier
//!    the catalogue lists short of Incognito: NEAR AI private, else
//!    External, else every model (owner, 2026-10-31). In that pool, the
//!    first of [`PRIVATE_PREFERRED`] that is listed; else Claude Code takes
//!    the newest Anthropic Sonnet and OpenCode the first chat model. Codex
//!    takes the first of [`CODEX_PREFERRED`] that is listed, from every
//!    tier, else the newest OpenAI GPT that is not in [`CODEX_REFUSED`].
//!    "Newest" is the version after the family prefix
//!    (`anthropic/claude-sonnet-5-5` is 5.5, `openai/gpt-6.1-sol` is 6.1);
//!    smaller variants (`mini`, `nano`, `lite`) are never the default.
//!    Without one: the family's first model, then the vendor's, then the
//!    first chat model.
//! 4. Every other model Claude Code is set to use (its family aliases and
//!    small, fast model) is of the default's tier or a more private one,
//!    else the default ([`claude_env`]). OpenCode's small model and its
//!    title and compaction agents are the default.

use super::{ManagedError, ToolId};
use serde::{Deserialize, Serialize};

/// Ids naming a non-chat model that NEAR AI describes as text in, text
/// out, or a non-chat model listed without modalities. Matched as
/// lowercase substrings.
pub const EXCLUDED: &[&str] = &["rerank", "privacy-filter", "embedding", "whisper", "flux"];

/// NEAR AI's privacy tier for a model, which a picker groups by. NEAR AI
/// labels each catalogue entry with one of three, through `owned_by`
/// (docs.near.ai/cloud/models, read 2026-10-30).
///
/// Declared most private first (owner, 2026-10-31), so the derived `Ord`
/// is the picker's order and [`ModelTier::ORDER`] lists it for a shell to
/// follow. Reordering the variants changes that order, not the wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// "Confidential TEE": NEAR AI's own GPUs, in a TEE. `owned_by` is
    /// `nearai`.
    NearAi,
    /// "3P Confidential TEE": a third party's TEE. `owned_by` is
    /// `Attested 3P`.
    External,
    /// "Incognito": a vendor's model (OpenAI, Anthropic, Google, ...) on
    /// the vendor's servers, sent through NEAR AI's gateway under NEAR AI's
    /// account. `owned_by` is the vendor's name. The default for any named
    /// owner not known to be a privacy provider, since it claims the least.
    Incognito,
    /// A tier a newer daemon sent that this build does not know. Treated as
    /// no tier: the picker lists the model last, under no heading.
    #[serde(other)]
    Unknown,
}

impl ModelTier {
    /// The tiers a picker groups by, most private first. `Unknown` is
    /// not one: it is listed last, under no heading.
    pub const ORDER: [Self; 3] = [Self::NearAi, Self::External, Self::Incognito];

    /// The tier `owned_by` names: NEAR AI or `Attested 3P` only when the
    /// owner says so, otherwise Incognito, the least private, whether the
    /// owner is unlisted, generic, blank or absent.
    fn from_owner(owned_by: Option<&str>) -> Self {
        let owner = owned_by.unwrap_or("").trim();
        let is = |name: &str| owner.eq_ignore_ascii_case(name);
        if ["nearai", "near-ai", "near ai"].into_iter().any(is) {
            Self::NearAi
        } else if is("attested 3p") {
            Self::External
        } else {
            Self::Incognito
        }
    }
}

/// A chat model from the NEAR AI catalogue, as a picker shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModel {
    pub id: String,
    /// NEAR AI's display name, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// NEAR AI's privacy tier. Incognito unless `owned_by` names a known
    /// private host; none only when a payload omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<ModelTier>,
    /// NEAR AI's context window for the model in tokens, input and output
    /// together (`context_length`). Each tool is told it, so it compacts
    /// before NEAR AI refuses a request as too long; none when NEAR AI
    /// gives none, and the tool keeps its own assumption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    /// The most tokens NEAR AI lets the model write in one answer
    /// (`max_output_length`); none when NEAR AI gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_length: Option<u64>,
}

impl ChatModel {
    /// The tokens a request may send and still leave room for the longest
    /// answer: the window less the output limit, as Codex sizes its own
    /// models (GPT-5: 400,000 less 128,000 is Codex's 272,000). The whole
    /// window when no output limit below it is given.
    pub fn input_window(&self) -> Option<u64> {
        let window = self.context_length?;
        Some(match self.max_output_length {
            Some(output) if output < window => window - output,
            _ => window,
        })
    }
}

/// The largest token count read from the catalogue; anything above is
/// treated as unreadable rather than passed to a tool.
const MAX_TOKENS: u64 = 100_000_000;

/// A positive token count in `value`, or none.
fn tokens(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().filter(|n| (1..=MAX_TOKENS).contains(n))
}

/// The chat models in a NEAR AI `/v1/models` answer, in its order. An id
/// is kept once, and only when it is safe to name in a tool's config.
pub fn chat_models(catalogue: &serde_json::Value) -> Vec<ChatModel> {
    let mut kept: Vec<ChatModel> = vec![];
    for entry in catalogue["data"].as_array().into_iter().flatten() {
        let Some(id) = entry["id"].as_str() else {
            continue;
        };
        if !super::opencode::valid_model(id)
            || kept.iter().any(|m| m.id == id)
            || !is_chat(entry, id)
        {
            continue;
        }
        kept.push(ChatModel {
            id: id.to_owned(),
            name: entry["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned),
            tier: Some(ModelTier::from_owner(entry["owned_by"].as_str())),
            context_length: tokens(&entry["context_length"])
                .or_else(|| tokens(&entry["top_provider"]["context_length"])),
            max_output_length: tokens(&entry["max_output_length"])
                .or_else(|| tokens(&entry["top_provider"]["max_completion_tokens"])),
        });
    }
    kept
}

fn is_chat(entry: &serde_json::Value, id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if EXCLUDED.iter().any(|word| lower.contains(word)) {
        return false;
    }
    let text = |value: &serde_json::Value| value.as_str() == Some("text");
    let input_ok = match entry["input_modalities"].as_array() {
        Some(input) => input.iter().any(text),
        None => entry.get("input_modalities").is_none(),
    };
    let output_ok = match entry["output_modalities"].as_array() {
        Some(output) => output.len() == 1 && text(&output[0]),
        None => entry.get("output_modalities").is_none(),
    };
    input_ok && output_ok
}

/// The family and vendor prefixes a tool prefers, most specific first.
fn preference(tool: ToolId) -> Option<(&'static str, &'static str)> {
    match tool {
        ToolId::Claude => Some(("anthropic/claude-sonnet-", "anthropic/")),
        ToolId::Codex => Some(("openai/gpt-", "openai/")),
        ToolId::Opencode => None,
    }
}

/// The version after a family prefix: its leading numeric parts (`5-5`,
/// `6.1-sol`); none for a smaller variant or a name with no number. A part
/// over three digits (a date stamp) ends the version.
fn version(rest: &str) -> Option<Vec<u32>> {
    let mut parts = rest.split(['.', '-']).peekable();
    let mut numbers = vec![];
    while let Some(part) =
        parts.next_if(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
    {
        numbers.push(part.parse().ok()?);
    }
    if numbers.is_empty() || parts.any(|p| matches!(p, "mini" | "nano" | "lite")) {
        return None;
    }
    Some(numbers)
}

/// The newest model of the family `prefix` (`anthropic/claude-opus-`):
/// the highest version, and among equals the first id in byte order.
/// Without a readable version, the family's first model.
fn newest_in<'a>(
    prefix: &str,
    mut models: impl Iterator<Item = &'a ChatModel> + Clone,
) -> Option<&'a ChatModel> {
    models
        .clone()
        .filter_map(|m| Some((version(m.id.strip_prefix(prefix)?)?, m)))
        .max_by(|(a, x), (b, y)| a.cmp(b).then_with(|| y.id.cmp(&x.id)))
        .map(|(_, m)| m)
        .or_else(|| models.find(|m| m.id.starts_with(prefix)))
}

/// Codex's default on NEAR AI, first listed first: models that answered a
/// Codex 0.163.0 request (its tools and a reasoning effort) through a
/// managed session's proxy on 2026-10-19. Newest is not enough for Codex,
/// because NEAR AI cannot run every GPT with Codex's tools; see
/// [`CODEX_REFUSED`].
pub const CODEX_PREFERRED: &[&str] = &["openai/gpt-6-sol", "openai/gpt-5.6-sol", "openai/gpt-5.5"];

/// Never offered to Codex ([`offered`]): NEAR AI refused each one a Codex
/// request on 2026-10-19, so it is left out of the picker and cannot be
/// chosen by name.
/// - `openai/gpt-6.1-sol`: "Function tools with reasoning_effort are not
///   supported for gpt-6.1-sol in /v1/chat/completions".
/// - `openai/gpt-6-astra`: "Native stateless Responses supports
///   client-managed function tools only".
pub const CODEX_REFUSED: &[&str] = &["openai/gpt-6.1-sol", "openai/gpt-6-astra"];

/// Claude Code's and OpenCode's default inside a NEAR AI private or
/// External pool, first listed first ([`default_for`]). NEAR AI lists
/// alphabetically, so without this list the default is whichever private
/// model sorts first, which may be a small, abliterated or vision model.
/// Each entry is a general chat model NEAR AI lists with `tools` in its
/// `supported_features` and a window of 128,000 tokens or more
/// (`/v1/models`, read 2026-10-31); none is abliterated or vision-only.
/// Not yet run with either tool's own tool schema: a product choice the
/// owner approved on 2026-10-31, not a measured one like
/// [`CODEX_PREFERRED`].
pub const PRIVATE_PREFERRED: &[&str] = &[
    // NEAR AI private.
    "Qwen/Qwen3.8-27B",
    "z-ai/glm-5.3",
    "deepseek-ai/DeepSeek-V4.1-Flash-1017",
    "z-ai/glm-5.3-flash",
    "Qwen/Qwen3.6-27B-FP8",
    // External.
    "moonshotai/kimi-k3",
    "z-ai/glm-5",
    "deepseek/deepseek-v3.2",
];

/// The chat models `tool` is offered, in NEAR AI's order: every one, less
/// [`CODEX_REFUSED`] for Codex. A session's picker lists these, and a
/// model chosen by name must be one of them.
pub fn offered(tool: ToolId, models: Vec<ChatModel>) -> Vec<ChatModel> {
    match tool {
        ToolId::Codex => models
            .into_iter()
            .filter(|m| !CODEX_REFUSED.contains(&m.id.as_str()))
            .collect(),
        ToolId::Claude | ToolId::Opencode => models,
    }
}

/// The model a tool starts on when none was chosen.
///
/// Claude Code and OpenCode never start on an Incognito model while the
/// catalogue lists a more private one (owner, 2026-10-31): the pool is the
/// models of the most private tier the catalogue has short of Incognito
/// ([`ModelTier`]'s order: NEAR AI private, then External), and every model
/// only when it has neither. Within the pool the first of
/// [`PRIVATE_PREFERRED`] that is listed, then the tool's own preference:
/// its family, then vendor, then first listed. Codex keeps to the models
/// NEAR AI is known to run with its tools ([`CODEX_PREFERRED`]), from every
/// tier. A model the person chose is never replaced ([`select`]).
pub fn default_for(tool: ToolId, models: &[ChatModel]) -> Option<&ChatModel> {
    let best = models
        .iter()
        .filter_map(|m| m.tier)
        .filter(|tier| *tier < ModelTier::Incognito)
        .min();
    match best {
        Some(best) if tool != ToolId::Codex => {
            let pool = models.iter().filter(move |m| m.tier == Some(best));
            listed(PRIVATE_PREFERRED, pool.clone()).or_else(|| preferred(tool, pool))
        }
        _ => preferred(tool, models.iter()),
    }
}

/// The first of `ids` that `models` holds.
fn listed<'a>(
    ids: &[&str],
    models: impl Iterator<Item = &'a ChatModel> + Clone,
) -> Option<&'a ChatModel> {
    ids.iter()
        .find_map(|id| models.clone().find(|m| m.id == *id))
}

/// `tool`'s own preference among `models`: Codex's list, then the family's
/// newest, then the vendor's first, then the first listed.
fn preferred<'a>(
    tool: ToolId,
    mut models: impl Iterator<Item = &'a ChatModel> + Clone,
) -> Option<&'a ChatModel> {
    let Some((family, vendor)) = preference(tool) else {
        return models.next();
    };
    if tool == ToolId::Codex {
        if let Some(model) = listed(CODEX_PREFERRED, models.clone()) {
            return Some(model);
        }
        let usable = models.filter(|m| !CODEX_REFUSED.contains(&m.id.as_str()));
        return pick_in(family, vendor, usable);
    }
    pick_in(family, vendor, models)
}

/// The family's newest, then the vendor's first, then the first listed.
fn pick_in<'a>(
    family: &str,
    vendor: &str,
    mut models: impl Iterator<Item = &'a ChatModel> + Clone,
) -> Option<&'a ChatModel> {
    newest_in(family, models.clone())
        .or_else(|| models.clone().find(|m| m.id.starts_with(vendor)))
        .or_else(|| models.next())
}

/// Claude Code's and OpenCode's background model by default in a NEAR AI
/// private pool, first listed first ([`background_default`]): the small,
/// fast chat models NEAR AI lists with `tools` (`/v1/models`, read
/// 2026-10-31). A product choice the owner approved on 2026-10-31, like
/// [`PRIVATE_PREFERRED`].
pub const BACKGROUND_PREFERRED: &[&str] = &[
    "z-ai/glm-5.3-flash",
    "Qwen/Qwen3.6-35B-A3B-FP8",
    "deepseek-ai/DeepSeek-V4.1-Flash-1017",
];

/// The tier `main` claims, when this build can read it: none for a tier
/// a newer daemon sent ([`ModelTier::Unknown`]) or a model with no tier.
fn known_tier(main: &ChatModel) -> Option<ModelTier> {
    main.tier.filter(|tier| *tier != ModelTier::Unknown)
}

/// Whether `model` may serve a session whose main model claims `ceiling`
/// ([`known_tier`]): a model of a known tier no less private than it. A
/// main model whose tier is unreadable claims nothing, so only `main_id`
/// itself passes (#1798 review: fail closed, never every model).
fn within(model: &ChatModel, ceiling: Option<ModelTier>, main_id: &str) -> bool {
    match ceiling {
        Some(ceiling) => known_tier(model).is_some_and(|tier| tier <= ceiling),
        None => model.id == main_id,
    }
}

/// The models that may serve `main`'s background work: those of `main`'s
/// tier or a more private one, in NEAR AI's order (owner, 2026-10-31). A
/// model with no known tier is never offered beside one that has one, and
/// a main model with no known tier is offered only itself.
pub fn background_options<'a>(main: &ChatModel, models: &'a [ChatModel]) -> Vec<&'a ChatModel> {
    let ceiling = known_tier(main);
    models
        .iter()
        .filter(|m| within(m, ceiling, &main.id))
        .collect()
}

/// `tool`'s background model for `main` when none was chosen: from
/// `main`'s own known tier, Claude Code's newest Haiku, else the first
/// listed of [`BACKGROUND_PREFERRED`], else `main` itself. A main model
/// with no known tier is its own background model.
pub fn background_default<'a>(
    tool: ToolId,
    main: &'a ChatModel,
    models: &'a [ChatModel],
) -> &'a ChatModel {
    known_tier(main)
        .and_then(|tier| tier_background(tool, tier, models))
        .unwrap_or(main)
}

/// The background default every main model of `tier` shares, when the
/// tier holds one: Claude Code's newest Haiku, else the first listed of
/// [`BACKGROUND_PREFERRED`]. None leaves each main model its own.
fn tier_background(tool: ToolId, tier: ModelTier, models: &[ChatModel]) -> Option<&ChatModel> {
    let same = models.iter().filter(move |m| m.tier == Some(tier));
    let haiku = match tool {
        ToolId::Claude => newest_in("anthropic/claude-haiku-", same.clone()),
        ToolId::Codex | ToolId::Opencode => None,
    };
    haiku.or_else(|| listed(BACKGROUND_PREFERRED, same))
}

/// What a background picker needs for every main model in `models`, from
/// the core so a shell keeps no privacy rule of its own (#1798 review):
/// `sets` holds each distinct list of allowed background ids
/// ([`background_options`]), and `models` maps each main model's id to its
/// background default ([`background_default`]) and the index of its set.
/// Worked out once per tier, not once per model. Empty for Codex, which
/// has no background slot.
pub fn background_table(tool: ToolId, models: &[ChatModel]) -> serde_json::Value {
    let mut sets: Vec<Vec<&str>> = vec![];
    let mut table = serde_json::Map::new();
    if tool != ToolId::Codex {
        let mut tiers: Vec<(ModelTier, usize, Option<&str>)> = vec![];
        for main in models {
            let (set, shared) = match known_tier(main) {
                Some(tier) => match tiers.iter().find(|(t, _, _)| *t == tier) {
                    Some((_, set, shared)) => (*set, *shared),
                    None => {
                        sets.push(
                            background_options(main, models)
                                .iter()
                                .map(|m| m.id.as_str())
                                .collect(),
                        );
                        let shared = tier_background(tool, tier, models).map(|m| m.id.as_str());
                        tiers.push((tier, sets.len() - 1, shared));
                        (sets.len() - 1, shared)
                    }
                },
                // No known tier: only itself, its own set.
                None => {
                    sets.push(vec![main.id.as_str()]);
                    (sets.len() - 1, None)
                }
            };
            table.insert(
                main.id.clone(),
                serde_json::json!({"default": shared.unwrap_or(&main.id), "set": set}),
            );
        }
    }
    serde_json::json!({"sets": sets, "models": table})
}

/// The background model a session of `tool` on `main` runs: `requested`
/// when it is one of [`background_options`], otherwise
/// [`background_default`], so a stale or hand-edited choice can never send
/// background work somewhere less private than the main model. None only
/// when `main` is not listed.
pub fn background_for<'a>(
    tool: ToolId,
    main: &str,
    requested: Option<&str>,
    models: &'a [ChatModel],
) -> Option<&'a ChatModel> {
    let main = models.iter().find(|m| m.id == main)?;
    let allowed = background_options(main, models);
    Some(
        requested
            .and_then(|id| allowed.iter().copied().find(|m| m.id == id))
            .unwrap_or_else(|| background_default(tool, main, models)),
    )
}

/// The model `tool`'s own flag names in `args`, the person's CLI options,
/// which overrides the session's default: Claude Code's `--model`, Codex's
/// `-m` or `--model`. The last one wins, as each tool reads them.
pub fn flag_model(tool: ToolId, args: &[String]) -> Option<&str> {
    let mut found = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let short = tool == ToolId::Codex;
        if arg == "--model" || (short && arg == "-m") {
            if let Some(value) = iter.next() {
                found = Some(value.as_str());
            }
        } else if let Some(value) = arg.strip_prefix("--model=") {
            found = Some(value);
        } else if let Some(value) = arg.strip_prefix("-m").filter(|v| short && !v.is_empty()) {
            found = Some(value);
        }
    }
    found.filter(|_| tool != ToolId::Opencode)
}

/// Claude Code's model aliases, each with the family whose newest listed
/// model it names. The Haiku alias names the background model instead.
const CLAUDE_ALIASES: [(&str, &str, &str); 4] = [
    (
        "anthropic/claude-opus-",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    ),
    (
        "anthropic/claude-sonnet-",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    ),
    (
        "anthropic/claude-haiku-",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    ),
    (
        "anthropic/claude-fable-",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    ),
];

/// The environment that makes every model Claude Code can ask for a model
/// NEAR AI lists, starting on `default`. A value of `None` removes the
/// variable, so one inherited from the person's shell cannot name an id
/// NEAR AI does not list or describe another model.
///
/// `ANTHROPIC_MODEL` is the default. Each family alias (Opus, Sonnet,
/// Haiku, Fable) names the family's newest listed model of the default's
/// tier or a more private one, with NEAR AI's display name when it gives
/// one; a family with no such model names the default, so the alias is
/// never given a substitute by the proxy and never sends Claude Code's
/// background or subagent requests to a less private model than the
/// session's own (owner, 2026-10-31).
/// The small, fast model and the Haiku alias are the background model
/// ([`background_for`]): `background` when it is no less private than the
/// default, else the tool's background default. A default that no alias
/// names (an older Sonnet, or another vendor's model while every family is
/// listed) is offered as the picker's custom entry. The default's
/// context window is `CLAUDE_CODE_MAX_CONTEXT_TOKENS`.
pub fn claude_env(
    default: &str,
    background: Option<&str>,
    models: &[ChatModel],
) -> Vec<(&'static str, Option<String>)> {
    let default_model = models.iter().find(|m| m.id == default);
    let named = |model: Option<&ChatModel>| model.and_then(|m| m.name.clone());
    // The models an alias may name: those at least as private as the
    // default, so no alias sends a request somewhere less private than the
    // session's own model. A default with no known tier claims nothing, so
    // every alias names the default; one the catalogue does not list, too.
    let ceiling = default_model.and_then(known_tier);
    let pool = models
        .iter()
        .filter(move |m| default_model.is_some() && within(m, ceiling, default));
    let background = background_for(ToolId::Claude, default, background, models);
    let mut env = vec![("ANTHROPIC_MODEL", Some(default.to_owned()))];
    let mut aliases = vec![];
    for (prefix, var, name_var) in CLAUDE_ALIASES {
        let pick = if prefix == "anthropic/claude-haiku-" {
            background
        } else {
            newest_in(prefix, pool.clone())
        };
        let (id, name) = match pick {
            Some(model) => (model.id.clone(), named(Some(model))),
            None => (default.to_owned(), named(default_model)),
        };
        env.push((var, Some(id.clone())));
        env.push((name_var, name));
        aliases.push(id);
    }
    // The background model, which the Haiku alias names: Claude Code reads
    // ANTHROPIC_SMALL_FAST_MODEL first and the Haiku alias after it for its
    // background work, and its Explore subagent asks for the Haiku alias.
    env.push(("ANTHROPIC_SMALL_FAST_MODEL", Some(aliases[2].clone())));
    let custom = !aliases.iter().any(|id| id == default);
    env.push((
        "ANTHROPIC_CUSTOM_MODEL_OPTION",
        custom.then(|| default.to_owned()),
    ));
    env.push((
        "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
        custom.then(|| named(default_model)).flatten(),
    ));
    // The default's window, for Claude Code to compact at when it cannot
    // size the model itself. Claude Code applies this only to an id it
    // does not recognize (another vendor's model, or a Claude model newer
    // than the installed Claude Code); a Claude model it knows keeps the
    // window it has on the Anthropic API. Removed when NEAR AI gives none,
    // so a value set for another gateway does not size this one.
    env.push((
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
        default_model
            .and_then(|m| m.context_length)
            .map(|n| n.to_string()),
    ));
    // Descriptions and capabilities a person set for other models, and a
    // subagent model that may not be listed: none of them apply here.
    for var in [
        "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_SUPPORTED_CAPABILITIES",
        "CLAUDE_CODE_SUBAGENT_MODEL",
        "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL",
        "CLAUDE_CODE_AUTO_MODE_MODEL",
        "CLAUDE_CODE_BG_CLASSIFIER_MODEL",
    ] {
        env.push((var, None));
    }
    env
}

/// The session's default model: the chosen one when it is a chat model in
/// the catalogue, the tool's default when none was chosen. Anything else
/// is refused, never replaced with another model.
pub fn select(
    tool: ToolId,
    requested: Option<&str>,
    models: &[ChatModel],
) -> Result<String, ManagedError> {
    match requested {
        Some(id) => models.iter().find(|m| m.id == id),
        None => default_for(tool, models),
    }
    .map(|m| m.id.clone())
    .ok_or(ManagedError::ModelUnavailable)
}

/// NEAR AI's chat models, read from `{base_url}/models`. Read directly
/// because the session proxy's own list carries ids only. Sent without a
/// key: the list is public, and a key goes only to the proxy. None when
/// NEAR AI could not be read.
pub async fn fetch(client: &reqwest::Client, base_url: &str) -> Option<Vec<ChatModel>> {
    let response = client
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    Some(chat_models(&response.json().await.ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A cut of NEAR AI's live catalogue (2026-10-16), in its order, with
    /// the rows that broke or would break the old first-entry default.
    fn catalogue() -> serde_json::Value {
        let text = |id: &str| json!({"id": id, "name": id, "input_modalities": ["text"], "output_modalities": ["text"]});
        json!({"object": "list", "data": [
            text("anthropic/claude-fable-5"),
            text("anthropic/claude-haiku-4-5"),
            text("anthropic/claude-opus-5-5"),
            text("anthropic/claude-sonnet-4-5-20250929"),
            text("anthropic/claude-sonnet-4-6"),
            text("anthropic/claude-sonnet-5"),
            text("anthropic/claude-sonnet-5-5"),
            {"id": "black-forest-labs/FLUX.2-klein-4B", "input_modalities": ["text"], "output_modalities": ["image"]},
            text("deepseek/deepseek-v3.2"),
            text("openai/gpt-4.1"),
            text("openai/gpt-5.5"),
            text("openai/gpt-5.6-sol"),
            text("openai/gpt-5-mini"),
            text("openai/gpt-6.1-mini"),
            text("openai/gpt-6.1-sol"),
            text("openai/gpt-6-astra"),
            text("openai/privacy-filter"),
            {"id": "openai/whisper-large-v3", "input_modalities": ["audio"], "output_modalities": ["text"]},
            {"id": "Qwen/Qwen3-Embedding-0.6B", "input_modalities": ["text"], "output_modalities": ["embedding"]},
            text("Qwen/Qwen3-Reranker-0.6B"),
            {"id": "typesafe/jev-1.13", "input_modalities": ["text"], "output_modalities": ["decisions"]},
            {"id": "z-ai/glm-5.3-flash", "input_modalities": ["text", "image"], "output_modalities": ["text"]},
        ]})
    }

    fn ids(models: &[ChatModel]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    fn models(list: &[&str]) -> Vec<ChatModel> {
        list.iter()
            .map(|id| ChatModel {
                id: (*id).into(),
                name: None,
                tier: None,
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn only_chat_models_are_kept_in_catalogue_order() {
        let kept = chat_models(&catalogue());
        assert_eq!(
            ids(&kept),
            vec![
                "anthropic/claude-fable-5",
                "anthropic/claude-haiku-4-5",
                "anthropic/claude-opus-5-5",
                "anthropic/claude-sonnet-4-5-20250929",
                "anthropic/claude-sonnet-4-6",
                "anthropic/claude-sonnet-5",
                "anthropic/claude-sonnet-5-5",
                "deepseek/deepseek-v3.2",
                "openai/gpt-4.1",
                "openai/gpt-5.5",
                "openai/gpt-5.6-sol",
                "openai/gpt-5-mini",
                "openai/gpt-6.1-mini",
                "openai/gpt-6.1-sol",
                "openai/gpt-6-astra",
                "z-ai/glm-5.3-flash",
            ]
        );
        assert_eq!(kept[0].name.as_deref(), Some("anthropic/claude-fable-5"));
    }

    /// NEAR AI's privacy tier comes from `owned_by`, as its catalogue
    /// labels it (live 2026-10-30): `nearai` runs in NEAR AI's TEEs,
    /// `Attested 3P` in a third party's, and a vendor's own name is a model
    /// on that vendor's servers; so is any owner not known to be private.
    #[test]
    fn each_model_carries_near_ai_privacy_tier() {
        let kept = chat_models(&json!({"data": [
            {"id": "anthropic/claude-sonnet-5-5", "owned_by": "anthropic"},
            {"id": "Qwen/Qwen3.8-27B", "owned_by": "nearai"},
            {"id": "z-ai/glm-5.3", "owned_by": "attested 3p"},
            {"id": "moonshotai/kimi-k3", "owned_by": "Attested 3P"},
            {"id": "x-ai/grok-4.7", "owned_by": "some-new-vendor"},
            {"id": "a/b", "owned_by": "system"},
            {"id": "c/d", "owned_by": "organization-owner"},
            {"id": "e/f", "owned_by": "NearAI"},
            {"id": "g/h", "owned_by": "Near-AI"},
            {"id": "i/j", "owned_by": "near ai"},
            {"id": "k/l", "owned_by": "OpenAI"},
            {"id": "no/owner"},
            {"id": "blank/owner", "owned_by": " "},
        ]}));
        let tiers: Vec<_> = kept.iter().map(|m| (m.id.as_str(), m.tier)).collect();
        assert_eq!(
            tiers,
            vec![
                ("anthropic/claude-sonnet-5-5", Some(ModelTier::Incognito)),
                ("Qwen/Qwen3.8-27B", Some(ModelTier::NearAi)),
                ("z-ai/glm-5.3", Some(ModelTier::External)),
                ("moonshotai/kimi-k3", Some(ModelTier::External)),
                ("x-ai/grok-4.7", Some(ModelTier::Incognito)),
                ("a/b", Some(ModelTier::Incognito)),
                ("c/d", Some(ModelTier::Incognito)),
                ("e/f", Some(ModelTier::NearAi)),
                ("g/h", Some(ModelTier::NearAi)),
                ("i/j", Some(ModelTier::NearAi)),
                ("k/l", Some(ModelTier::Incognito)),
                ("no/owner", Some(ModelTier::Incognito)),
                ("blank/owner", Some(ModelTier::Incognito)),
            ]
        );
        // On the wire: snake_case, and absent when unknown.
        assert_eq!(
            serde_json::to_value(&kept[..3]).unwrap(),
            json!([
                {"id": "anthropic/claude-sonnet-5-5", "tier": "incognito"},
                {"id": "Qwen/Qwen3.8-27B", "tier": "near_ai"},
                {"id": "z-ai/glm-5.3", "tier": "external"},
            ])
        );
        assert_eq!(
            serde_json::to_value(&kept[11]).unwrap(),
            json!({"id": "no/owner", "tier": "incognito"})
        );
    }

    /// A tier string a newer daemon sends reads as `Unknown`, not an error,
    /// so an older launcher still loads the launch context.
    #[test]
    fn unknown_tier_string_deserializes() {
        let model: ChatModel =
            serde_json::from_value(json!({"id": "a/b", "tier": "future_tier"})).unwrap();
        assert_eq!(model.tier, Some(ModelTier::Unknown));
    }

    /// An entry without modality fields is kept unless its id names a
    /// non-chat model; an entry without a usable id never is.
    #[test]
    fn entries_without_modalities_fall_back_to_the_id_rule() {
        let kept = chat_models(&json!({"data": [
            {"id": "Qwen/Qwen3-Embedding-0.6B"},
            {"id": "openai/whisper-large-v3"},
            {"id": "deepseek-ai/DeepSeek-V3.1"},
            {"id": "deepseek-ai/DeepSeek-V3.1"},
            {"id": ""},
            {"id": "has space"},
            {"name": "no id"},
            {"id": 7},
        ]}));
        assert_eq!(ids(&kept), vec!["deepseek-ai/DeepSeek-V3.1"]);
        assert!(chat_models(&json!({"error": "nope"})).is_empty());
    }

    fn tiered(list: &[(&str, ModelTier)]) -> Vec<ChatModel> {
        list.iter()
            .map(|(id, tier)| ChatModel {
                id: (*id).into(),
                tier: Some(*tier),
                ..Default::default()
            })
            .collect()
    }

    /// The tiers sort most private first, whatever order they are named
    /// in, and `ORDER` lists the same sequence for a shell to follow.
    #[test]
    fn tiers_order_most_private_first() {
        let mut shuffled = [
            ModelTier::Unknown,
            ModelTier::Incognito,
            ModelTier::NearAi,
            ModelTier::External,
        ];
        shuffled.sort();
        assert_eq!(
            shuffled,
            [
                ModelTier::NearAi,
                ModelTier::External,
                ModelTier::Incognito,
                ModelTier::Unknown,
            ]
        );
        assert_eq!(
            ModelTier::ORDER,
            [ModelTier::NearAi, ModelTier::External, ModelTier::Incognito]
        );
        assert_eq!(
            serde_json::to_value(ModelTier::ORDER).unwrap(),
            json!(["near_ai", "external", "incognito"])
        );
    }

    /// #1798 review: the macOS shell keeps one copy of the tier order, a
    /// fallback for a daemon from before `tier_order`. It must name the
    /// same tiers in the same order as [`ModelTier::ORDER`].
    #[test]
    fn the_shells_fallback_tier_order_is_the_cores() {
        let swift =
            include_str!("../../../../macos/Sources/TCShellCore/ManagedSessionsSurface.swift");
        let line = swift
            .lines()
            .find(|line| line.contains("static let knownTierOrder"))
            .expect("the shell's fallback order");
        let literal = &line[line.find('[').unwrap()..=line.rfind(']').unwrap()];
        let shell: Vec<String> = serde_json::from_str(literal).unwrap();
        assert_eq!(
            serde_json::to_value(shell).unwrap(),
            serde_json::to_value(ModelTier::ORDER).unwrap()
        );
    }

    /// With a NEAR AI private (TEE-hosted) model listed, Claude and
    /// OpenCode start on it, the first of [`PRIVATE_PREFERRED`]; Codex keeps
    /// to the models NEAR AI is known to run with its tools.
    #[test]
    fn the_default_prefers_a_near_ai_private_model() {
        let listed = tiered(&[
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("z-ai/glm-5.3", ModelTier::External),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
            ("Qwen/Qwen3.8-9B", ModelTier::NearAi),
            ("openai/gpt-6-sol", ModelTier::Incognito),
        ]);
        for tool in [ToolId::Claude, ToolId::Opencode] {
            assert_eq!(default_for(tool, &listed).unwrap().id, "Qwen/Qwen3.8-27B");
        }
        assert_eq!(
            default_for(ToolId::Codex, &listed).unwrap().id,
            "openai/gpt-6-sol"
        );
    }

    /// With no NEAR AI private model, a third party's TEE model is the
    /// default over an Incognito one; with neither, the default is what it
    /// was: the tool's family, then vendor, then first. A model with no
    /// tier is not private.
    #[test]
    fn the_default_is_never_incognito_while_a_private_or_external_model_is_listed() {
        let listed = tiered(&[
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("x/incognito", ModelTier::Incognito),
            ("z-ai/glm-5.3", ModelTier::External),
        ]);
        for tool in [ToolId::Claude, ToolId::Opencode] {
            assert_eq!(default_for(tool, &listed).unwrap().id, "z-ai/glm-5.3");
        }
        // Within a tier the tool's own preference decides.
        let two = tiered(&[
            ("a/first", ModelTier::NearAi),
            ("anthropic/claude-sonnet-5-5", ModelTier::NearAi),
            ("anthropic/claude-sonnet-6", ModelTier::Incognito),
        ]);
        assert_eq!(
            default_for(ToolId::Claude, &two).unwrap().id,
            "anthropic/claude-sonnet-5-5"
        );
        assert_eq!(default_for(ToolId::Opencode, &two).unwrap().id, "a/first");
    }

    #[test]
    fn the_default_falls_back_without_a_near_ai_private_model() {
        let listed = tiered(&[
            ("z-ai/glm-5.3", ModelTier::External),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
        ]);
        assert_eq!(
            default_for(ToolId::Claude, &listed).unwrap().id,
            "z-ai/glm-5.3"
        );
        assert_eq!(
            default_for(ToolId::Opencode, &listed).unwrap().id,
            "z-ai/glm-5.3"
        );
        // Incognito only: the tool's own preference, as before.
        let incognito = tiered(&[
            ("x/first", ModelTier::Incognito),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
        ]);
        assert_eq!(
            default_for(ToolId::Claude, &incognito).unwrap().id,
            "anthropic/claude-sonnet-5-5"
        );
        let untiered = models(&["x/first", "anthropic/claude-sonnet-5-5"]);
        assert_eq!(
            default_for(ToolId::Claude, &untiered).unwrap().id,
            "anthropic/claude-sonnet-5-5"
        );
        assert_eq!(default_for(ToolId::Claude, &[]), None);
    }

    /// Only the default moves: a model the contributor chose is kept,
    /// private model listed or not.
    #[test]
    fn a_chosen_model_is_kept_over_a_private_default() {
        let listed = tiered(&[
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
        ]);
        assert_eq!(
            select(ToolId::Claude, Some("anthropic/claude-sonnet-5-5"), &listed),
            Ok("anthropic/claude-sonnet-5-5".into())
        );
        assert_eq!(
            select(ToolId::Claude, None, &listed),
            Ok("Qwen/Qwen3.8-27B".into())
        );
    }

    #[test]
    fn each_tool_defaults_to_its_own_family_newest_first() {
        let kept = chat_models(&catalogue());
        let default = |tool| default_for(tool, &kept).map(|m| m.id.as_str());
        assert_eq!(default(ToolId::Claude), Some("anthropic/claude-sonnet-5-5"));
        assert_eq!(default(ToolId::Codex), Some("openai/gpt-5.6-sol"));
        assert_eq!(default(ToolId::Opencode), Some("anthropic/claude-fable-5"));
    }

    #[test]
    fn defaults_fall_back_family_then_vendor_then_first() {
        // A family member with no readable version is still the family.
        let family = models(&[
            "x/first",
            "anthropic/claude-opus-5",
            "anthropic/claude-sonnet-next",
        ]);
        assert_eq!(
            default_for(ToolId::Claude, &family).unwrap().id,
            "anthropic/claude-sonnet-next"
        );
        // Only small variants: still the family, never another vendor.
        let small = models(&["x/first", "openai/o3", "openai/gpt-5-mini"]);
        assert_eq!(
            default_for(ToolId::Codex, &small).unwrap().id,
            "openai/gpt-5-mini"
        );
        let vendor = models(&["x/first", "anthropic/claude-opus-5", "openai/o3"]);
        assert_eq!(
            default_for(ToolId::Claude, &vendor).unwrap().id,
            "anthropic/claude-opus-5"
        );
        assert_eq!(default_for(ToolId::Codex, &vendor).unwrap().id, "openai/o3");
        let other = models(&["x/first", "y/second"]);
        assert_eq!(default_for(ToolId::Claude, &other).unwrap().id, "x/first");
        assert_eq!(default_for(ToolId::Codex, &[]), None);
    }

    /// Codex starts on a model NEAR AI answered with Codex's tools, never
    /// on one it refused, however new.
    #[test]
    fn codex_defaults_to_a_model_near_ai_runs_with_its_tools() {
        let all = models(&[
            "openai/gpt-6.1-sol",
            "openai/gpt-6-astra",
            "openai/gpt-5.5",
            "openai/gpt-6-sol",
        ]);
        assert_eq!(
            default_for(ToolId::Codex, &all).unwrap().id,
            "openai/gpt-6-sol"
        );
        // The next preferred when the first is gone.
        let later = models(&["openai/gpt-6.1-sol", "openai/gpt-5.5"]);
        assert_eq!(
            default_for(ToolId::Codex, &later).unwrap().id,
            "openai/gpt-5.5"
        );
        // None preferred: the newest GPT that is not refused.
        let rest = models(&["openai/gpt-6.1-sol", "openai/gpt-6-astra", "openai/gpt-5.4"]);
        assert_eq!(
            default_for(ToolId::Codex, &rest).unwrap().id,
            "openai/gpt-5.4"
        );
        // Only refused models: another vendor's, then nothing refused.
        let refused = models(&["openai/gpt-6.1-sol", "x/first"]);
        assert_eq!(default_for(ToolId::Codex, &refused).unwrap().id, "x/first");
    }

    /// Codex is never offered a model NEAR AI refused it: not in its
    /// picker, not by name. Other tools are offered every chat model.
    #[test]
    fn codex_is_not_offered_a_refused_model() {
        let all = models(&[
            "openai/gpt-6.1-sol",
            "openai/gpt-6-astra",
            "openai/gpt-6-sol",
            "anthropic/claude-sonnet-5-5",
        ]);
        let codex = offered(ToolId::Codex, all.clone());
        assert_eq!(
            ids(&codex),
            vec!["openai/gpt-6-sol", "anthropic/claude-sonnet-5-5"]
        );
        assert_eq!(
            select(ToolId::Codex, Some("openai/gpt-6.1-sol"), &codex),
            Err(ManagedError::ModelUnavailable)
        );
        for tool in [ToolId::Claude, ToolId::Opencode] {
            assert_eq!(offered(tool, all.clone()), all);
        }
    }

    /// Equal versions pick the same model whatever the catalogue order.
    #[test]
    fn equal_versions_pick_deterministically() {
        let a = models(&[
            "openai/gpt-7-sol",
            "openai/gpt-7-astra",
            "openai/gpt-7-luna",
        ]);
        let b = models(&[
            "openai/gpt-7-luna",
            "openai/gpt-7-sol",
            "openai/gpt-7-astra",
        ]);
        assert_eq!(
            default_for(ToolId::Codex, &a).unwrap().id,
            "openai/gpt-7-astra"
        );
        assert_eq!(
            default_for(ToolId::Codex, &b).unwrap().id,
            "openai/gpt-7-astra"
        );
    }

    #[test]
    fn a_chosen_model_must_be_a_listed_chat_model() {
        let kept = chat_models(&catalogue());
        assert_eq!(
            select(ToolId::Claude, Some("anthropic/claude-opus-5-5"), &kept),
            Ok("anthropic/claude-opus-5-5".into())
        );
        // Off-family is allowed when chosen.
        assert_eq!(
            select(ToolId::Codex, Some("z-ai/glm-5.3-flash"), &kept),
            Ok("z-ai/glm-5.3-flash".into())
        );
        for refused in [
            "anthropic/claude-sonnet-9",
            "Qwen/Qwen3-Embedding-0.6B",
            "openai/privacy-filter",
            "",
        ] {
            assert_eq!(
                select(ToolId::Claude, Some(refused), &kept),
                Err(ManagedError::ModelUnavailable),
                "{refused}"
            );
        }
        assert_eq!(
            select(ToolId::Claude, None, &kept),
            Ok("anthropic/claude-sonnet-5-5".into())
        );
        assert_eq!(
            select(ToolId::Codex, None, &[]),
            Err(ManagedError::ModelUnavailable)
        );
    }

    /// NEAR AI's window and output limit are read from each entry, from
    /// `top_provider` when the top level lacks them; zero, negative, huge
    /// or non-numeric values are unreadable, never passed on.
    #[test]
    fn context_limits_are_read_from_the_catalogue() {
        let text = json!({"input_modalities": ["text"], "output_modalities": ["text"]});
        let entry = |id: &str, extra: serde_json::Value| {
            let mut e = text.clone();
            e["id"] = json!(id);
            for (k, v) in extra.as_object().unwrap() {
                e[k] = v.clone();
            }
            e
        };
        let kept = chat_models(&json!({"data": [
            entry("a/top", json!({"context_length": 1_050_000, "max_output_length": 128_000})),
            entry("b/provider", json!({"top_provider": {"context_length": 262_144, "max_completion_tokens": 65_536}})),
            entry("c/none", json!({})),
            entry("d/bad", json!({"context_length": 0, "max_output_length": "big"})),
            entry("e/huge", json!({"context_length": 1u64 << 40, "max_output_length": -1})),
        ]}));
        let limits: Vec<_> = kept
            .iter()
            .map(|m| (m.id.as_str(), m.context_length, m.max_output_length))
            .collect();
        assert_eq!(
            limits,
            vec![
                ("a/top", Some(1_050_000), Some(128_000)),
                ("b/provider", Some(262_144), Some(65_536)),
                ("c/none", None, None),
                ("d/bad", None, None),
                ("e/huge", None, None),
            ]
        );
        assert_eq!(kept[0].input_window(), Some(922_000));
        assert_eq!(kept[2].input_window(), None);
        // A window that arrived without these fields still deserializes.
        let old: ChatModel = serde_json::from_value(json!({"id": "x/y"})).unwrap();
        assert_eq!(old.context_length, None);
        assert!(
            serde_json::to_value(&old)
                .unwrap()
                .get("context_length")
                .is_none()
        );
    }

    /// Claude Code is told the default's window, which it uses for a model
    /// it cannot size itself; without one, an inherited value is removed.
    #[test]
    fn claude_is_told_the_default_models_context_window() {
        let sized = |id: &str, window: Option<u64>| ChatModel {
            id: id.into(),
            context_length: window,
            ..Default::default()
        };
        let kept = vec![
            sized("anthropic/claude-sonnet-5-5", Some(1_000_000)),
            sized("z-ai/glm-5.3", Some(1_048_576)),
            sized("x/unsized", None),
        ];
        let vars = claude_env("z-ai/glm-5.3", None, &kept);
        assert_eq!(
            env_of(&vars)["CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
            Some("1048576")
        );
        let vars = claude_env("anthropic/claude-sonnet-5-5", None, &kept);
        assert_eq!(
            env_of(&vars)["CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
            Some("1000000")
        );
        let vars = claude_env("x/unsized", None, &kept);
        let env = env_of(&vars);
        assert!(env.contains_key("CLAUDE_CODE_MAX_CONTEXT_TOKENS"));
        assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], None);
    }

    fn env_of<'a>(
        vars: &'a [(&'static str, Option<String>)],
    ) -> std::collections::BTreeMap<&'static str, Option<&'a str>> {
        vars.iter().map(|(k, v)| (*k, v.as_deref())).collect()
    }

    /// Each Claude Code alias names the newest listed model of its family,
    /// with NEAR AI's display name, so every pick reaches NEAR AI as named.
    #[test]
    fn claude_aliases_name_the_newest_listed_model_of_each_family() {
        let mut kept = chat_models(&catalogue());
        kept.push(ChatModel {
            id: "anthropic/claude-opus-4-1".into(),
            name: Some("Claude Opus 4.1".into()),
            tier: None,
            ..Default::default()
        });
        kept[2].name = Some("Claude Opus 5.5".into());
        let vars = claude_env("anthropic/claude-sonnet-5-5", None, &kept);
        let env = env_of(&vars);
        assert_eq!(env["ANTHROPIC_MODEL"], Some("anthropic/claude-sonnet-5-5"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            Some("anthropic/claude-opus-5-5")
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"],
            Some("Claude Opus 5.5")
        );
        // 5.5 is newer than 5 and than 4.6; a date stamp ends a version.
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            Some("anthropic/claude-sonnet-5-5")
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            Some("anthropic/claude-haiku-4-5")
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_FABLE_MODEL"],
            Some("anthropic/claude-fable-5")
        );
        assert_eq!(
            env["ANTHROPIC_SMALL_FAST_MODEL"],
            Some("anthropic/claude-haiku-4-5")
        );
        // The default is one of the aliases: no extra picker entry, and an
        // inherited one is cleared, as is a subagent model naming an id
        // NEAR AI may not list.
        assert_eq!(env["ANTHROPIC_CUSTOM_MODEL_OPTION"], None);
        assert_eq!(env["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"], None);
        assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"], None);
        assert!(env.contains_key("CLAUDE_CODE_SUBAGENT_MODEL"));
        // Every id set is a listed chat model.
        for (name, value) in &vars {
            if name.ends_with("_MODEL") || name.ends_with("_OPTION") {
                if let Some(id) = value {
                    assert!(kept.iter().any(|m| &m.id == id), "{name}={id}");
                }
            }
        }
    }

    /// Every model Claude Code is set to use comes from the default's tier
    /// or a more private one (owner, 2026-10-31): with a NEAR AI private
    /// default, no alias, small/fast model or custom entry names an
    /// Incognito model, so no background or subagent request leaves the
    /// TEE. A family the pool lacks names the default.
    #[test]
    fn claude_never_names_a_less_private_model_than_its_default() {
        let listed = tiered(&[
            ("anthropic/claude-haiku-4-5", ModelTier::Incognito),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("anthropic/claude-opus-5-5", ModelTier::Incognito),
            ("anthropic/claude-fable-5", ModelTier::Incognito),
            ("z-ai/glm-5", ModelTier::External),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
        ]);
        let default = select(ToolId::Claude, None, &listed).unwrap();
        assert_eq!(default, "Qwen/Qwen3.8-27B");
        let tier_of = |id: &str| listed.iter().find(|m| m.id == id).and_then(|m| m.tier);
        let vars = claude_env(&default, None, &listed);
        for (name, value) in &vars {
            if let Some(id) = value {
                assert_ne!(tier_of(id), Some(ModelTier::Incognito), "{name}={id}");
                assert_ne!(tier_of(id), Some(ModelTier::External), "{name}={id}");
            }
        }
        let env = env_of(&vars);
        for var in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
        ] {
            assert_eq!(env[var], Some("Qwen/Qwen3.8-27B"), "{var}");
        }
        assert_eq!(env["ANTHROPIC_CUSTOM_MODEL_OPTION"], None);

        // An External default may use a NEAR AI private model, never an
        // Incognito one.
        let vars = claude_env("z-ai/glm-5", None, &listed);
        for (name, value) in &vars {
            if let Some(id) = value {
                assert_ne!(tier_of(id), Some(ModelTier::Incognito), "{name}={id}");
            }
        }

        // A family the pool does hold is still named by its newest model.
        let pooled = tiered(&[
            ("anthropic/claude-haiku-4-5", ModelTier::Incognito),
            ("anthropic/claude-haiku-5-5", ModelTier::NearAi),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
        ]);
        let env_vars = claude_env("Qwen/Qwen3.8-27B", None, &pooled);
        let env = env_of(&env_vars);
        assert_eq!(
            env["ANTHROPIC_SMALL_FAST_MODEL"],
            Some("anthropic/claude-haiku-5-5")
        );

        // A person who chose an Incognito model keeps every family alias.
        let vars = claude_env("anthropic/claude-sonnet-5-5", None, &listed);
        let env = env_of(&vars);
        assert_eq!(
            env["ANTHROPIC_SMALL_FAST_MODEL"],
            Some("anthropic/claude-haiku-4-5")
        );
    }

    /// A model the person chose holds Claude Code to its tier just as the
    /// default does: an explicit NEAR AI private pick, with Incognito
    /// Anthropic models listed, leaves no variable naming an Incognito
    /// model, and a stale Incognito background choice is replaced.
    #[test]
    fn an_explicit_private_pick_keeps_every_claude_variable_private() {
        let listed = tiered(&[
            ("anthropic/claude-haiku-4-5", ModelTier::Incognito),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("anthropic/claude-opus-5-5", ModelTier::Incognito),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
            ("z-ai/glm-5.3", ModelTier::NearAi),
            ("z-ai/glm-5.3-flash", ModelTier::NearAi),
        ]);
        let tier_of = |id: &str| listed.iter().find(|m| m.id == id).and_then(|m| m.tier);
        let chosen = select(ToolId::Claude, Some("z-ai/glm-5.3"), &listed).unwrap();
        for background in [None, Some("anthropic/claude-haiku-4-5"), Some("gone/model")] {
            let vars = claude_env(&chosen, background, &listed);
            for (name, value) in &vars {
                if let Some(id) = value {
                    assert_eq!(
                        tier_of(id),
                        Some(ModelTier::NearAi),
                        "{name}={id} {background:?}"
                    );
                }
            }
            let env = env_of(&vars);
            assert_eq!(env["ANTHROPIC_MODEL"], Some("z-ai/glm-5.3"));
            // The background default in the NEAR AI private tier.
            assert_eq!(
                env["ANTHROPIC_SMALL_FAST_MODEL"],
                Some("z-ai/glm-5.3-flash")
            );
            assert_eq!(
                env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
                Some("z-ai/glm-5.3-flash")
            );
        }
        // A background choice in the tier is kept, for both variables.
        let vars = claude_env(&chosen, Some("Qwen/Qwen3.8-27B"), &listed);
        let env = env_of(&vars);
        assert_eq!(env["ANTHROPIC_SMALL_FAST_MODEL"], Some("Qwen/Qwen3.8-27B"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            Some("Qwen/Qwen3.8-27B")
        );
        // Only an Incognito main model lets the others be Incognito.
        let vars = claude_env("anthropic/claude-sonnet-5-5", None, &listed);
        let env = env_of(&vars);
        assert_eq!(
            env["ANTHROPIC_SMALL_FAST_MODEL"],
            Some("anthropic/claude-haiku-4-5")
        );
        // Model variables an inherited shell could set are removed.
        for var in [
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL",
            "CLAUDE_CODE_AUTO_MODE_MODEL",
            "CLAUDE_CODE_BG_CLASSIFIER_MODEL",
        ] {
            assert!(env.contains_key(var), "{var}");
            assert_eq!(env[var], None, "{var}");
        }
    }

    /// The background model may be the main model's tier or a more private
    /// one, never less private (owner, 2026-10-31). Its default is the
    /// tool's small model in the main model's own tier, else the main
    /// model; a choice outside the allowed set is replaced by that default.
    #[test]
    fn the_background_model_is_never_less_private_than_the_main_model() {
        let listed = tiered(&[
            ("anthropic/claude-haiku-4-5", ModelTier::Incognito),
            ("anthropic/claude-haiku-5", ModelTier::Incognito),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("z-ai/glm-5", ModelTier::External),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
            ("z-ai/glm-5.3-flash", ModelTier::NearAi),
        ]);
        let ids = |models: Vec<&ChatModel>| models.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        let main = |id: &str| listed.iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            ids(background_options(main("Qwen/Qwen3.8-27B"), &listed)),
            vec!["Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash"]
        );
        assert_eq!(
            ids(background_options(main("z-ai/glm-5"), &listed)),
            vec!["z-ai/glm-5", "Qwen/Qwen3.8-27B", "z-ai/glm-5.3-flash"]
        );
        assert_eq!(
            background_options(main("anthropic/claude-sonnet-5-5"), &listed).len(),
            6
        );
        // Defaults: newest Haiku of the main's tier for Claude, the small
        // private model in a private tier, else the main model.
        let default = |tool, id: &str| background_default(tool, main(id), &listed).id.as_str();
        assert_eq!(
            default(ToolId::Claude, "anthropic/claude-sonnet-5-5"),
            "anthropic/claude-haiku-5"
        );
        assert_eq!(
            default(ToolId::Claude, "Qwen/Qwen3.8-27B"),
            "z-ai/glm-5.3-flash"
        );
        assert_eq!(
            default(ToolId::Opencode, "Qwen/Qwen3.8-27B"),
            "z-ai/glm-5.3-flash"
        );
        assert_eq!(default(ToolId::Claude, "z-ai/glm-5"), "z-ai/glm-5");
        assert_eq!(
            default(ToolId::Opencode, "anthropic/claude-sonnet-5-5"),
            "anthropic/claude-sonnet-5-5"
        );
        // A choice is kept only inside the allowed set.
        let pick = |tool, id: &str, requested| {
            background_for(tool, id, requested, &listed).map(|m| m.id.as_str())
        };
        assert_eq!(
            pick(ToolId::Claude, "z-ai/glm-5", Some("Qwen/Qwen3.8-27B")),
            Some("Qwen/Qwen3.8-27B")
        );
        assert_eq!(
            pick(ToolId::Claude, "Qwen/Qwen3.8-27B", Some("z-ai/glm-5")),
            Some("z-ai/glm-5.3-flash")
        );
        assert_eq!(
            pick(
                ToolId::Claude,
                "Qwen/Qwen3.8-27B",
                Some("anthropic/claude-haiku-5")
            ),
            Some("z-ai/glm-5.3-flash")
        );
        assert_eq!(
            pick(ToolId::Opencode, "Qwen/Qwen3.8-27B", Some("gone/model")),
            Some("z-ai/glm-5.3-flash")
        );
        assert_eq!(pick(ToolId::Claude, "gone/main", None), None);
        // A model with no tier is not private: never offered beside a
        // private main model.
        let mixed = vec![
            ChatModel {
                id: "n/main".into(),
                tier: Some(ModelTier::NearAi),
                ..Default::default()
            },
            ChatModel {
                id: "x/untiered".into(),
                ..Default::default()
            },
        ];
        assert_eq!(ids(background_options(&mixed[0], &mixed)), vec!["n/main"]);
        // An untiered main model claims nothing: only itself (#1798
        // review), never every model.
        assert_eq!(
            ids(background_options(&mixed[1], &mixed)),
            vec!["x/untiered"]
        );
    }

    /// #1798 review: a main model whose tier this build cannot read (a
    /// newer tier, or a payload without one) claims nothing, so every
    /// secondary model is the main model itself, never an Incognito one,
    /// and a main model the catalogue does not list names only itself.
    #[test]
    fn secondary_models_fail_closed_when_the_main_tier_is_unknown_or_missing() {
        let mut listed = tiered(&[
            ("anthropic/claude-haiku-4-5", ModelTier::Incognito),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
            ("anthropic/claude-opus-5-5", ModelTier::Incognito),
            ("anthropic/claude-fable-5", ModelTier::Incognito),
            ("z-ai/glm-5.3-flash", ModelTier::NearAi),
        ]);
        let future: ChatModel =
            serde_json::from_value(json!({"id": "x/future", "tier": "some_future_tier"})).unwrap();
        assert_eq!(future.tier, Some(ModelTier::Unknown));
        listed.push(future);
        listed.push(ChatModel {
            id: "x/untiered".into(),
            ..Default::default()
        });
        for main in ["x/future", "x/untiered", "x/not-listed"] {
            for background in [
                None,
                Some("anthropic/claude-haiku-4-5"),
                Some("z-ai/glm-5.3-flash"),
            ] {
                let vars = claude_env(main, background, &listed);
                for (name, value) in &vars {
                    if let Some(id) = value {
                        assert_eq!(id, main, "{name} {background:?}");
                    }
                }
            }
            if main != "x/not-listed" {
                let model = listed.iter().find(|m| m.id == main).unwrap();
                let ids: Vec<&str> = background_options(model, &listed)
                    .into_iter()
                    .map(|m| m.id.as_str())
                    .collect();
                assert_eq!(ids, vec![main]);
                assert_eq!(background_default(ToolId::Claude, model, &listed).id, main);
                assert_eq!(
                    background_for(
                        ToolId::Opencode,
                        main,
                        Some("anthropic/claude-haiku-4-5"),
                        &listed
                    )
                    .map(|m| m.id.as_str()),
                    Some(main)
                );
            }
        }
    }

    /// The model a tool's own flag names in the person's CLI options: it
    /// overrides the default, so the other models follow it instead.
    #[test]
    fn the_model_flag_in_cli_options_is_the_model_in_use() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            flag_model(ToolId::Codex, &args(&["-m", "a/b"])),
            Some("a/b")
        );
        assert_eq!(
            flag_model(ToolId::Codex, &args(&["--model", "a/b"])),
            Some("a/b")
        );
        assert_eq!(
            flag_model(ToolId::Codex, &args(&["--model=a/b"])),
            Some("a/b")
        );
        assert_eq!(flag_model(ToolId::Codex, &args(&["-ma/b"])), Some("a/b"));
        assert_eq!(
            flag_model(ToolId::Codex, &args(&["-m", "a/b", "-m", "c/d"])),
            Some("c/d")
        );
        assert_eq!(
            flag_model(ToolId::Claude, &args(&["--model", "a/b"])),
            Some("a/b")
        );
        assert_eq!(
            flag_model(ToolId::Claude, &args(&["--model=a/b"])),
            Some("a/b")
        );
        // Claude's -m is not a model flag; a missing value names nothing.
        assert_eq!(flag_model(ToolId::Claude, &args(&["-m", "a/b"])), None);
        assert_eq!(flag_model(ToolId::Codex, &args(&["--model"])), None);
        assert_eq!(
            flag_model(ToolId::Codex, &args(&["--sandbox", "workspace-write"])),
            None
        );
        assert_eq!(
            flag_model(ToolId::Opencode, &args(&["--model", "a/b"])),
            None
        );
    }

    /// Inside a private pool Claude Code and OpenCode start on a model in
    /// [`PRIVATE_PREFERRED`], whatever NEAR AI's alphabetical order puts
    /// first; a pool holding none of them falls back to the first listed.
    #[test]
    fn a_private_pool_prefers_a_listed_agent_model_over_the_first() {
        let listed = tiered(&[
            (
                "deepseek-ai/DeepSeek-V4.1-Flash-1017-abliterated",
                ModelTier::NearAi,
            ),
            ("meta-llama/llama-3.3-70b-instruct", ModelTier::NearAi),
            ("Qwen/Qwen3.8-9B", ModelTier::NearAi),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
        ]);
        for tool in [ToolId::Claude, ToolId::Opencode] {
            assert_eq!(default_for(tool, &listed).unwrap().id, PRIVATE_PREFERRED[0]);
        }
        // The list is walked in order: the first one listed wins.
        let later = tiered(&[
            ("a/unlisted", ModelTier::NearAi),
            (PRIVATE_PREFERRED[2], ModelTier::NearAi),
            (PRIVATE_PREFERRED[1], ModelTier::NearAi),
        ]);
        assert_eq!(
            default_for(ToolId::Opencode, &later).unwrap().id,
            PRIVATE_PREFERRED[1]
        );
        // External pool: its own entries in the list.
        let external = tiered(&[
            ("a/unlisted", ModelTier::External),
            ("moonshotai/kimi-k3", ModelTier::External),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
        ]);
        assert_eq!(
            default_for(ToolId::Claude, &external).unwrap().id,
            "moonshotai/kimi-k3"
        );
        // No entry listed: the first in NEAR AI's order, as before.
        let none = tiered(&[
            ("a/first", ModelTier::NearAi),
            ("b/second", ModelTier::NearAi),
        ]);
        assert_eq!(default_for(ToolId::Claude, &none).unwrap().id, "a/first");
        // No entry names an abliterated or a `-vl-` (vision) variant. Tool
        // support and the window come from NEAR AI's catalogue, as the
        // list's doc says; an id cannot show them, so this checks ids only.
        for id in PRIVATE_PREFERRED {
            let lower = id.to_ascii_lowercase();
            assert!(
                !lower.contains("abliterated") && !lower.contains("-vl-"),
                "{id}"
            );
        }
        // Codex is unchanged: its own list, across every tier.
        let codex = tiered(&[
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
            ("openai/gpt-6-sol", ModelTier::Incognito),
        ]);
        assert_eq!(
            default_for(ToolId::Codex, &codex).unwrap().id,
            "openai/gpt-6-sol"
        );
    }

    /// A family NEAR AI does not list maps to the default model, so the
    /// alias is served as the default rather than tier-picked; a default
    /// outside the aliases is offered as its own picker entry.
    #[test]
    fn missing_families_map_to_the_default_and_an_off_family_default_is_offered() {
        let kept = vec![
            ChatModel {
                id: "openai/gpt-6.1-sol".into(),
                name: Some("GPT-6.1 Sol".into()),
                tier: Some(ModelTier::Incognito),
                ..Default::default()
            },
            ChatModel {
                id: "anthropic/claude-sonnet-5".into(),
                name: None,
                tier: Some(ModelTier::Incognito),
                ..Default::default()
            },
        ];
        let vars = claude_env("openai/gpt-6.1-sol", None, &kept);
        let env = env_of(&vars);
        assert_eq!(env["ANTHROPIC_MODEL"], Some("openai/gpt-6.1-sol"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            Some("anthropic/claude-sonnet-5")
        );
        // No display name from NEAR AI: none set, and an inherited one is
        // cleared so it cannot describe another model.
        assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"], None);
        assert!(env.contains_key("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"));
        for family in ["OPUS", "HAIKU", "FABLE"] {
            let var = format!("ANTHROPIC_DEFAULT_{family}_MODEL");
            assert_eq!(env[var.as_str()], Some("openai/gpt-6.1-sol"), "{var}");
            assert_eq!(
                env[format!("{var}_NAME").as_str()],
                Some("GPT-6.1 Sol"),
                "{var}"
            );
        }
        assert_eq!(
            env["ANTHROPIC_SMALL_FAST_MODEL"],
            Some("openai/gpt-6.1-sol")
        );
        // The default already is an alias (Opus, here): no second entry.
        assert_eq!(env["ANTHROPIC_CUSTOM_MODEL_OPTION"], None);
        // With every family listed, an OpenAI default is no alias's model,
        // so it is offered as the picker's custom entry.
        let mut kept = chat_models(&catalogue());
        kept[13].name = Some("GPT-6.1 Sol".into());
        assert_eq!(kept[13].id, "openai/gpt-6.1-sol");
        let vars = claude_env("openai/gpt-6.1-sol", None, &kept);
        let env = env_of(&vars);
        assert_eq!(env["ANTHROPIC_MODEL"], Some("openai/gpt-6.1-sol"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            Some("anthropic/claude-opus-5-5")
        );
        assert_eq!(
            env["ANTHROPIC_CUSTOM_MODEL_OPTION"],
            Some("openai/gpt-6.1-sol")
        );
        assert_eq!(
            env["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"],
            Some("GPT-6.1 Sol")
        );
        // An older Sonnet chosen as the default is not an alias either.
        let vars = claude_env("anthropic/claude-sonnet-4-6", None, &kept);
        let env = env_of(&vars);
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            Some("anthropic/claude-sonnet-5-5")
        );
        assert_eq!(
            env["ANTHROPIC_CUSTOM_MODEL_OPTION"],
            Some("anthropic/claude-sonnet-4-6")
        );
        assert_eq!(
            env["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"],
            Some("anthropic/claude-sonnet-4-6")
        );
    }
}
