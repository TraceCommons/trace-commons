//! Which NEAR AI model a managed session runs on.
//!
//! The session's proxy pins one model, so it must be chosen on purpose:
//! NEAR AI lists its catalogue alphabetically, and the first entry is
//! neither a sensible default for every tool nor always a chat model.
//!
//! The rule, kept here so a later mode that pins only the backend can use
//! the same default:
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
//!    Codex the newest OpenAI GPT. "Newest" is the version after the family
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

/// A chat model from the NEAR AI catalogue, as a picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModel {
    pub id: String,
    /// NEAR AI's display name, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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

/// The model `tool` runs on when none was chosen.
pub fn default_for(tool: ToolId, models: &[ChatModel]) -> Option<&ChatModel> {
    let Some((family, vendor)) = preference(tool) else {
        return models.first();
    };
    models
        .iter()
        .filter_map(|m| Some((version(m.id.strip_prefix(family)?)?, m)))
        // Newest version; among equals, the first id in byte order.
        .max_by(|(a, x), (b, y)| a.cmp(b).then_with(|| y.id.cmp(&x.id)))
        .map(|(_, m)| m)
        .or_else(|| models.iter().find(|m| m.id.starts_with(family)))
        .or_else(|| models.iter().find(|m| m.id.starts_with(vendor)))
        .or_else(|| models.first())
}

/// The model to pin: the chosen one when it is a chat model in the
/// catalogue, the tool's default when none was chosen. Anything else is
/// refused, never replaced with another model.
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
    /// the rows that broke or would break the old first-entry pin.
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

    #[test]
    fn each_tool_defaults_to_its_own_family_newest_first() {
        let kept = chat_models(&catalogue());
        let default = |tool| default_for(tool, &kept).map(|m| m.id.as_str());
        assert_eq!(default(ToolId::Claude), Some("anthropic/claude-sonnet-5-5"));
        assert_eq!(default(ToolId::Codex), Some("openai/gpt-6.1-sol"));
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

    /// Equal versions pick the same model whatever the catalogue order.
    #[test]
    fn equal_versions_pick_deterministically() {
        let a = models(&[
            "openai/gpt-6-sol",
            "openai/gpt-6-astra",
            "openai/gpt-6-luna",
        ]);
        let b = models(&[
            "openai/gpt-6-luna",
            "openai/gpt-6-sol",
            "openai/gpt-6-astra",
        ]);
        assert_eq!(
            default_for(ToolId::Codex, &a).unwrap().id,
            "openai/gpt-6-astra"
        );
        assert_eq!(
            default_for(ToolId::Codex, &b).unwrap().id,
            "openai/gpt-6-astra"
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
}
