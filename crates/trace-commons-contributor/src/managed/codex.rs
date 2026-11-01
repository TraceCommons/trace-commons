//! Codex's model list for a managed NEAR AI session.
//!
//! The session's proxy serves any model NEAR AI lists as named, so Codex is
//! given a model catalog (`-c model_catalog_json=<file>`) naming every
//! NEAR AI chat model, the session's default first. Codex 0.163.0 takes
//! that catalog as the whole list: its picker offers those models only,
//! and it fetches no other list.
//!
//! A catalog entry also carries the settings and prompt Codex runs a model
//! with. Without a catalog Codex finds those in the list it ships with,
//! matching a NEAR AI id such as `openai/gpt-6.1-sol` to its own
//! `gpt-6.1-sol`. To keep that, the launcher reads the shipped list
//! (`codex debug models --bundled`) and a model Codex matches keeps its
//! prompt and its reasoning and context settings under the NEAR AI id
//! ([`KEPT`]). Every entry's context window is at most what NEAR AI
//! lists for the model, less its output limit
//! ([`ChatModel::input_window`]). Every entry's tool settings are basic, because the
//! shipped ones make Codex send requests NEAR AI refuses. Any other
//! model gets basic settings with the prompt of Codex's own first listed
//! model.
use super::{ManagedError, models::ChatModel};
use crate::config::ConfigStore;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The prompt every entry carries when Codex's own list could not be read.
pub const BASIC_INSTRUCTIONS: &str = "You are a coding agent.";

/// The config keys naming a model Codex uses besides the session's own,
/// each set to the model in use at launch and refused in the person's CLI
/// options: the reviewer of `/review`, the default subagent model, and the
/// memory jobs' models. Each is a string key of Codex 0.166.0's config,
/// which refuses a non-string value at that path (`codex debug models -c
/// <key>=1`, read 2026-10-31).
pub const SECONDARY_MODEL_KEYS: &[&str] = &[
    "review_model",
    "agents.default_subagent_model",
    "memories.extract_model",
    "memories.consolidation_model",
];

/// Where the launcher keeps each session's catalog. The launcher owns it:
/// a sign-in session's proxy home belongs to the daemon.
pub const CATALOGS: &str = "managed-codex-catalogs";

/// Codex's model catalog for a session: every NEAR AI chat model, the
/// default first, each shown in the picker under NEAR AI's display name
/// or its id. `bundled` is Codex's own list (`codex debug models
/// --bundled`); without it, or in another shape, every entry is basic.
pub fn catalog(default: &str, models: &[ChatModel], bundled: Option<&Value>) -> Value {
    let shipped: &[Value] = bundled
        .and_then(|b| b["models"].as_array())
        .filter(|list| list.iter().all(|m| m["slug"].is_string()))
        .map(Vec::as_slice)
        .unwrap_or_default();
    // The prompt of Codex's own first listed model, as it would use for a
    // model it does not know.
    let prompt = shipped
        .iter()
        .filter(|m| m["visibility"] == "list")
        .filter(|m| {
            m["base_instructions"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        })
        .min_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX))
        .and_then(|m| m["base_instructions"].as_str())
        .unwrap_or(BASIC_INSTRUCTIONS);
    let ordered = models
        .iter()
        .filter(|m| m.id == default)
        .take(1)
        .chain(models.iter().filter(|m| m.id != default));
    let entries: Vec<Value> = ordered
        .enumerate()
        .map(|(index, model)| {
            let display = model.name.as_deref().unwrap_or(&model.id);
            let mut entry = json!({
                "supported_reasoning_levels": [],
                "shell_type": "shell_command",
                "support_verbosity": false,
                "truncation_policy": {"mode": "bytes", "limit": 10000},
                "experimental_supported_tools": [],
                "base_instructions": prompt,
                "upgrade": null,
                "use_responses_lite": false,
            });
            if let Some(shipped) = known(&model.id, shipped) {
                for field in KEPT {
                    if let Some(value) = shipped.get(*field) {
                        entry[*field] = value.clone();
                    }
                }
            }
            // Never more than NEAR AI serves: the window it lists less the
            // longest answer, which is how Codex sizes its own models, and
            // no more than the window Codex ships for a model it knows.
            if let Some(near) = model.input_window() {
                let window = entry["context_window"]
                    .as_u64()
                    .map_or(near, |shipped| shipped.min(near));
                entry["context_window"] = json!(window);
            }
            entry["slug"] = json!(model.id);
            // Auto mode's reviewer would otherwise ask for Codex's hidden
            // `codex-auto-review`, which NEAR AI does not serve: it reviews
            // with the model it reviews for, in the same tier.
            entry["auto_review_model_override"] = json!(model.id);
            entry["display_name"] = json!(display);
            entry["visibility"] = json!("list");
            entry["supported_in_api"] = json!(true);
            entry["priority"] = json!(index + 1);
            entry
        })
        .collect();
    json!({ "models": entries })
}

/// What a model Codex knows keeps of its shipped entry: its prompt and
/// its reasoning and context settings. Nothing that shapes the tools:
/// those settings make Codex send what NEAR AI's Responses API refuses
/// (checked against it with Codex 0.163.0, 2026-10-19):
/// `use_responses_lite` moves the tools into an `additional_tools` item of
/// `input` ("data did not match any variant of untagged enum
/// ResponseInput"), and `apply_patch_tool_type` and `tool_mode` add
/// `custom` grammar tools ("unknown variant `custom`").
const KEPT: &[&str] = &[
    "base_instructions",
    "supported_reasoning_levels",
    "default_reasoning_level",
    "support_verbosity",
    "default_verbosity",
    "truncation_policy",
    "context_window",
    "effective_context_window_percent",
    "input_modalities",
];

/// The shipped entry Codex would use for `id`: the longest slug `id`
/// starts with, then the same after one `namespace/` (Codex 0.163.0,
/// `construct_model_info_from_candidates`).
fn known<'a>(id: &str, shipped: &'a [Value]) -> Option<&'a Value> {
    let longest = |name: &str| {
        shipped
            .iter()
            .filter_map(|m| Some((m["slug"].as_str()?, m)))
            .filter(|(slug, _)| name.starts_with(slug))
            .max_by_key(|(slug, _)| slug.len())
            .map(|(_, m)| m)
    };
    longest(id).or_else(|| {
        let (namespace, suffix) = id.split_once('/')?;
        let simple = !namespace.is_empty()
            && namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        (simple && !suffix.contains('/'))
            .then(|| longest(suffix))
            .flatten()
    })
}

/// Codex's own model list, read from the installed binary with `command`
/// (the profile's Codex, with `debug models --bundled` appended). None
/// when it cannot be read: the catalog is then basic, never refused.
pub fn bundled(command: std::process::Command) -> Option<Value> {
    let output = super::profiles::bounded_output_limit(command, 16 * 1024 * 1024).ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// A session's catalog file, removed when dropped: when the session ends,
/// and on any early return before the tool starts.
pub struct CatalogFile {
    path: PathBuf,
}

impl CatalogFile {
    /// Write `catalog` for session `id` under the launcher's own state.
    pub fn write(store: &ConfigStore, id: Uuid, catalog: &Value) -> Result<Self, ManagedError> {
        let root = store.dir().join(CATALOGS);
        super::profiles::private_directory(&root)?;
        let name = format!("{id}.json");
        let body = serde_json::to_vec(catalog).map_err(|_| ManagedError::InvalidRequest)?;
        ConfigStore::open(root.clone())
            .and_then(|dir| dir.write_daemon_file(&name, &body))
            .map_err(|_| ManagedError::StorageUnavailable)?;
        Ok(Self {
            path: root.join(name),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CatalogFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The fields Codex 0.163.0 requires of every catalog entry.
    const REQUIRED: &[&str] = &[
        "slug",
        "display_name",
        "supported_reasoning_levels",
        "shell_type",
        "visibility",
        "supported_in_api",
        "priority",
        "support_verbosity",
        "truncation_policy",
        "experimental_supported_tools",
        "base_instructions",
    ];

    fn chat(id: &str, name: Option<&str>) -> ChatModel {
        ChatModel {
            id: id.into(),
            name: name.map(Into::into),
            tier: None,
            ..Default::default()
        }
    }

    /// A cut of `codex debug models --bundled` (0.163.0): two listed
    /// entries and a hidden one, prompts shortened.
    fn bundled() -> Value {
        let entry = |slug: &str, priority: u32, visibility: &str, prompt: &str| {
            json!({
                "slug": slug, "display_name": slug.to_uppercase(),
                "description": "Bundled.", "default_reasoning_level": "low",
                "supported_reasoning_levels": [{"effort": "low", "description": "Fast"}],
                "shell_type": "unified_exec", "visibility": visibility,
                "supported_in_api": true, "priority": priority,
                "availability_nux": {"message": "Try it."},
                "upgrade": {"model": "gpt-6-sol", "migration_markdown": "Meet GPT-6 Sol"},
                "support_verbosity": true,
                "truncation_policy": {"mode": "tokens", "limit": 10000},
                "experimental_supported_tools": ["clock"],
                "base_instructions": prompt,
                "model_messages": {"instructions_template": prompt},
                "use_responses_lite": true,
                "tool_mode": "code_mode_only",
                "apply_patch_tool_type": "freeform",
                "context_window": 272000,
            })
        };
        json!({"models": [
            entry("gpt-6-astra", 2, "list", "You are Codex (astra)."),
            entry("gpt-6.1-sol", 1, "list", "You are Codex (sol)."),
            entry("gpt-5.5", 13, "list", "You are Codex (5.5)."),
            entry("codex-auto-review", 0, "hide", "Review."),
        ]})
    }

    fn slugs(catalog: &Value) -> Vec<&str> {
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["slug"].as_str().unwrap())
            .collect()
    }

    /// Every NEAR AI chat model is listed once, the default first, each
    /// with every field Codex requires, shown in the picker and ordered.
    #[test]
    fn every_chat_model_is_listed_default_first_with_the_required_fields() {
        let models = [
            chat("anthropic/claude-sonnet-5-5", Some("Claude Sonnet 5.5")),
            chat("openai/gpt-6.1-sol", None),
            chat("openai/gpt-5.5-pro", Some("GPT-5.5 Pro")),
            chat("z-ai/glm-5.3-flash", None),
        ];
        for bundled in [Some(bundled()), None] {
            let catalog = catalog("openai/gpt-6.1-sol", &models, bundled.as_ref());
            assert_eq!(
                slugs(&catalog),
                [
                    "openai/gpt-6.1-sol",
                    "anthropic/claude-sonnet-5-5",
                    "openai/gpt-5.5-pro",
                    "z-ai/glm-5.3-flash",
                ]
            );
            for (index, entry) in catalog["models"].as_array().unwrap().iter().enumerate() {
                for field in REQUIRED {
                    assert!(entry.get(field).is_some(), "{field} in {entry}");
                }
                assert_eq!(entry["visibility"], "list");
                assert_eq!(entry["supported_in_api"], true);
                assert_eq!(entry["priority"], index as u64 + 1);
                assert!(
                    entry["base_instructions"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                );
                // No entry points Codex at a model NEAR AI does not list.
                assert!(entry["upgrade"].is_null(), "{entry}");
            }
            let names: Vec<&str> = catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["display_name"].as_str().unwrap())
                .collect();
            assert_eq!(
                names,
                [
                    "openai/gpt-6.1-sol",
                    "Claude Sonnet 5.5",
                    "GPT-5.5 Pro",
                    "z-ai/glm-5.3-flash"
                ]
            );
        }
    }

    /// Each entry's window is at most NEAR AI's, less the output limit:
    /// a model Codex knows keeps a smaller shipped window, a larger one is
    /// cut to NEAR AI's, and a model Codex does not know is given NEAR AI's.
    /// A model NEAR AI gives no window for keeps whatever Codex would use.
    #[test]
    fn context_windows_never_exceed_what_near_ai_lists() {
        let sized = |id: &str, window: u64, output: Option<u64>| ChatModel {
            context_length: Some(window),
            max_output_length: output,
            ..chat(id, None)
        };
        let models = [
            sized("openai/gpt-6.1-sol", 1_050_000, Some(128_000)),
            sized("openai/gpt-5.5", 200_000, Some(64_000)),
            sized("z-ai/glm-5.3-flash", 1_048_576, None),
            sized("x/small", 32_768, Some(65_536)),
            chat("x/unsized", None),
        ];
        let bundled = bundled();
        let catalog = catalog("openai/gpt-6.1-sol", &models, Some(&bundled));
        let window = |slug: &str| {
            catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["slug"] == slug)
                .unwrap()["context_window"]
                .clone()
        };
        assert_eq!(window("openai/gpt-6.1-sol"), 272_000);
        assert_eq!(window("openai/gpt-5.5"), 136_000);
        assert_eq!(window("z-ai/glm-5.3-flash"), 1_048_576);
        assert_eq!(window("x/small"), 32_768);
        assert!(window("x/unsized").is_null());
    }

    /// A model Codex knows keeps Codex's own settings and prompt, as it
    /// would without a catalog; any other model gets the basic settings
    /// with Codex's default prompt; without Codex's list, a short prompt.
    #[test]
    fn known_models_keep_codex_settings_and_others_get_its_default_prompt() {
        let models = [
            chat("openai/gpt-6.1-sol", None),
            chat("openai/gpt-5.5-pro", None),
            chat("anthropic/claude-sonnet-5-5", None),
            chat("gpt-6-astra", None),
        ];
        let bundled = bundled();
        let catalog = catalog("openai/gpt-6.1-sol", &models, Some(&bundled));
        let entry = |slug: &str| {
            catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["slug"] == slug)
                .unwrap()
                .clone()
        };
        // Namespaced id, Codex's own slug after the namespace.
        let sol = entry("openai/gpt-6.1-sol");
        assert_eq!(sol["base_instructions"], "You are Codex (sol).");
        assert_eq!(sol["supported_reasoning_levels"][0]["effort"], "low");
        assert_eq!(sol["context_window"], 272000);
        // Its tool settings are the basic ones, not Codex's.
        assert_eq!(sol["shell_type"], "shell_command");
        assert_eq!(sol["experimental_supported_tools"], json!([]));
        // Longest prefix, as Codex matches it.
        assert_eq!(
            entry("openai/gpt-5.5-pro")["base_instructions"],
            "You are Codex (5.5)."
        );
        assert_eq!(
            entry("gpt-6-astra")["base_instructions"],
            "You are Codex (astra)."
        );
        // Unknown to Codex: basic settings, the prompt of Codex's own
        // first listed model.
        let claude = entry("anthropic/claude-sonnet-5-5");
        assert_eq!(claude["base_instructions"], "You are Codex (sol).");
        assert_eq!(claude["shell_type"], "shell_command");
        assert_eq!(claude["supported_reasoning_levels"], json!([]));
        assert!(claude.get("model_messages").is_none());
        // Without Codex's list every entry is basic.
        let plain = super::catalog("openai/gpt-6.1-sol", &models, None);
        assert_eq!(plain["models"][0]["shell_type"], "shell_command");
        assert_eq!(plain["models"][0]["base_instructions"], BASIC_INSTRUCTIONS);
        // A list that is not Codex's shape counts as none.
        let odd = super::catalog("openai/gpt-6.1-sol", &models, Some(&json!({"models": "x"})));
        assert_eq!(odd, plain);
    }

    /// No entry carries Codex's tool settings: NEAR AI refuses the "lite"
    /// request (`use_responses_lite`) and `custom` grammar tools
    /// (`apply_patch_tool_type`, `tool_mode`). Codex's own list turns them
    /// on for every OpenAI model it ships, so a NEAR AI id it matches would
    /// otherwise inherit them.
    /// Auto mode's reviewer reviews with each entry's own model, never
    /// Codex's hidden `codex-auto-review`, which NEAR AI does not serve.
    #[test]
    fn every_entry_reviews_with_its_own_model() {
        let models = [chat("a/one", None), chat("b/two", Some("Two"))];
        let catalog = catalog("b/two", &models, Some(&bundled()));
        let entries = catalog["models"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert_eq!(entry["auto_review_model_override"], entry["slug"]);
        }
    }

    #[test]
    fn no_entry_carries_codex_tool_settings() {
        let models = [
            chat("openai/gpt-6.1-sol", None),
            chat("openai/gpt-6-astra", None),
            chat("anthropic/claude-sonnet-5-5", None),
        ];
        let bundled = bundled();
        for list in [Some(&bundled), None] {
            let catalog = catalog("openai/gpt-6.1-sol", &models, list);
            for entry in catalog["models"].as_array().unwrap() {
                assert_eq!(entry["use_responses_lite"], false, "{}", entry["slug"]);
            }
        }
        // A known entry is the basic one plus what it keeps: none of the
        // settings that change the tools Codex sends.
        let catalog = catalog("openai/gpt-6.1-sol", &models, Some(&bundled));
        let keys = |entry: &Value| {
            let mut keys: Vec<String> = entry.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        let mut expected: Vec<String> = [
            "slug",
            "display_name",
            "visibility",
            "supported_in_api",
            "priority",
            "shell_type",
            "experimental_supported_tools",
            "upgrade",
            "use_responses_lite",
            "auto_review_model_override",
        ]
        .iter()
        .chain(KEPT)
        .filter(|field| bundled["models"][1].get(**field).is_some() || !KEPT.contains(field))
        .map(|field| field.to_string())
        .collect();
        expected.sort();
        expected.dedup();
        assert_eq!(keys(&catalog["models"][0]), expected);
        for field in ["tool_mode", "apply_patch_tool_type", "model_messages"] {
            assert!(catalog["models"][0].get(field).is_none(), "{field}");
        }
    }

    /// The catalog file lives under the launcher's own state, readable by
    /// its owner only, and is gone once the session drops it.
    #[test]
    fn the_catalog_file_is_private_and_removed_when_dropped() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().into()).unwrap();
        let id = Uuid::new_v4();
        let value = catalog("x/a", &[chat("x/a", None)], None);
        let file = CatalogFile::write(&store, id, &value).unwrap();
        let path = file.path().to_owned();
        assert!(path.is_absolute());
        assert_eq!(path, store.dir().join(CATALOGS).join(format!("{id}.json")));
        let read: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, value);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        drop(file);
        assert!(!path.exists());
    }
}
