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
//! 3. Otherwise, per tool: Claude Code takes the newest Anthropic Sonnet,
//!    Codex the first of [`CODEX_PREFERRED`] that is listed, else the
//!    newest OpenAI GPT that is not in [`CODEX_REFUSED`]. "Newest" is the version after the family
//!    prefix (`anthropic/claude-sonnet-5-5` is 5.5, `openai/gpt-6.1-sol` is
//!    6.1); smaller variants (`mini`, `nano`, `lite`) are never the default.
//!    Without one: the family's first model, then the vendor's, then the
//!    first chat model. OpenCode takes the first chat model.

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
fn newest_in<'a>(prefix: &str, models: &'a [ChatModel]) -> Option<&'a ChatModel> {
    models
        .iter()
        .filter_map(|m| Some((version(m.id.strip_prefix(prefix)?)?, m)))
        .max_by(|(a, x), (b, y)| a.cmp(b).then_with(|| y.id.cmp(&x.id)))
        .map(|(_, m)| m)
        .or_else(|| models.iter().find(|m| m.id.starts_with(prefix)))
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
/// Claude Code and OpenCode start on a NEAR AI private (TEE-hosted) model
/// when the catalogue lists one, the first in NEAR AI's order, and
/// otherwise on the tool's family, then vendor, then first listed. Codex
/// keeps to the models NEAR AI is known to run with its tools
/// ([`CODEX_PREFERRED`]). A model the person chose is never replaced
/// ([`select`]).
pub fn default_for(tool: ToolId, models: &[ChatModel]) -> Option<&ChatModel> {
    if tool != ToolId::Codex {
        if let Some(private) = models.iter().find(|m| m.tier == Some(ModelTier::NearAi)) {
            return Some(private);
        }
    }
    let Some((family, vendor)) = preference(tool) else {
        return models.first();
    };
    if tool == ToolId::Codex {
        if let Some(model) = CODEX_PREFERRED
            .iter()
            .find_map(|id| models.iter().find(|m| m.id == *id))
        {
            return Some(model);
        }
        let usable: Vec<ChatModel> = models
            .iter()
            .filter(|m| !CODEX_REFUSED.contains(&m.id.as_str()))
            .cloned()
            .collect();
        let chosen = newest_in(family, &usable)
            .or_else(|| usable.iter().find(|m| m.id.starts_with(vendor)))
            .or_else(|| usable.first())?;
        return models.iter().find(|m| m.id == chosen.id);
    }
    newest_in(family, models)
        .or_else(|| models.iter().find(|m| m.id.starts_with(vendor)))
        .or_else(|| models.first())
}

/// Claude Code's model aliases, each with the family whose newest listed
/// model it names.
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
/// Haiku, Fable) names the family's newest listed model, with NEAR AI's
/// display name when it gives one; a family NEAR AI does not list names
/// the default, so the alias is never given a substitute by the proxy.
/// The small, fast model is the Haiku alias's. A default that no alias
/// names (an older Sonnet, or another vendor's model while every family is
/// listed) is offered as the picker's custom entry. The default's
/// context window is `CLAUDE_CODE_MAX_CONTEXT_TOKENS`.
pub fn claude_env(default: &str, models: &[ChatModel]) -> Vec<(&'static str, Option<String>)> {
    let default_model = models.iter().find(|m| m.id == default);
    let named = |model: Option<&ChatModel>| model.and_then(|m| m.name.clone());
    let mut env = vec![("ANTHROPIC_MODEL", Some(default.to_owned()))];
    let mut aliases = vec![];
    for (prefix, var, name_var) in CLAUDE_ALIASES {
        let (id, name) = match newest_in(prefix, models) {
            Some(model) => (model.id.clone(), named(Some(model))),
            None => (default.to_owned(), named(default_model)),
        };
        env.push((var, Some(id.clone())));
        env.push((name_var, name));
        aliases.push(id);
    }
    // The Haiku alias, third in the table.
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

    /// With a NEAR AI private (TEE-hosted) model listed, Claude and
    /// OpenCode start on it, the first one in NEAR AI's order; Codex keeps
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

    /// With no NEAR AI private model, the default is what it was: the
    /// tool's family, then vendor, then first. A third party's TEE is not
    /// NEAR AI's own, and a model with no tier is not private.
    #[test]
    fn the_default_falls_back_without_a_near_ai_private_model() {
        let listed = tiered(&[
            ("z-ai/glm-5.3", ModelTier::External),
            ("anthropic/claude-sonnet-5-5", ModelTier::Incognito),
        ]);
        assert_eq!(
            default_for(ToolId::Claude, &listed).unwrap().id,
            "anthropic/claude-sonnet-5-5"
        );
        assert_eq!(
            default_for(ToolId::Opencode, &listed).unwrap().id,
            "z-ai/glm-5.3"
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
        let vars = claude_env("z-ai/glm-5.3", &kept);
        assert_eq!(
            env_of(&vars)["CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
            Some("1048576")
        );
        let vars = claude_env("anthropic/claude-sonnet-5-5", &kept);
        assert_eq!(
            env_of(&vars)["CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
            Some("1000000")
        );
        let vars = claude_env("x/unsized", &kept);
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
        let vars = claude_env("anthropic/claude-sonnet-5-5", &kept);
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

    /// A family NEAR AI does not list maps to the default model, so the
    /// alias is served as the default rather than tier-picked; a default
    /// outside the aliases is offered as its own picker entry.
    #[test]
    fn missing_families_map_to_the_default_and_an_off_family_default_is_offered() {
        let kept = vec![
            ChatModel {
                id: "openai/gpt-6.1-sol".into(),
                name: Some("GPT-6.1 Sol".into()),
                tier: None,
                ..Default::default()
            },
            ChatModel {
                id: "anthropic/claude-sonnet-5".into(),
                name: None,
                tier: None,
                ..Default::default()
            },
        ];
        let vars = claude_env("openai/gpt-6.1-sol", &kept);
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
        let vars = claude_env("openai/gpt-6.1-sol", &kept);
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
        let vars = claude_env("anthropic/claude-sonnet-4-6", &kept);
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
