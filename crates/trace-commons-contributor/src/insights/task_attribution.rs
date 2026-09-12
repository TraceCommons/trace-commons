//! Bounded source-turn evidence from a pinned Codex rollout writer profile.
//!
//! This classifier reads explicitly supplied bytes and retains only hashes,
//! declared model labels, fixed profile metadata, counts, and record
//! coordinates. It does not retain prompts, responses, paths, or raw session
//! and turn identifiers. A source turn is not an independent user work item;
//! multiple source turns can later belong to one `LocalComparisonTask`. The
//! legacy profile remains fixture-only. The released profile is bound to the
//! exact Codex 0.154.0 writer revision and a finite direct/reasoning/sequential
//! exec record contract. It qualifies writer structure and declared cohorts;
//! it does not prove live provider identity, exclusive model serving, outcome,
//! or independence between user-reviewed tasks.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 262_144;
const MAX_TURNS: usize = 1_024;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexTaskSourceProfile {
    PinnedDirectWriterFixtureC4017a87,
    CodexRustV0_154_0TaskRecords,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualificationScope {
    WriterFixtureOnly,
    ReleasedWriterTaskRecords,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceProfileEvidence {
    pub profile: CodexTaskSourceProfile,
    pub profile_id: String,
    pub upstream_revision: String,
    pub observed_workspace_version: String,
    pub qualification_scope: QualificationScope,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskAttributionUnavailableReason {
    SourceProfileMismatch,
    UnsupportedRecord,
    InvalidRecordOrder,
    MissingRequiredRecord,
    IdentityConflict,
    ForkOrDelegation,
    ModelConflict,
    ContextConflict,
    ConfigurationConflict,
    RecordLimitExceeded,
    TurnLimitExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum TaskAttributionState {
    Attributed {
        turns: Vec<AttributedTurn>,
    },
    Unavailable {
        reason: TaskAttributionUnavailableReason,
        record_index: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageRecordStatus {
    Valid,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TurnRecordRefs {
    pub task_started: u64,
    pub permissions_context: Option<u64>,
    pub environment_context: Option<u64>,
    pub world_state: Option<u64>,
    pub turn_context: u64,
    pub user_text: u64,
    pub user_event: u64,
    /// Qualified reasoning/tool/usage records between the user and final answer.
    #[serde(default)]
    pub intermediate_records: Vec<u64>,
    #[serde(default)]
    pub intermediate_usage_records: Vec<u64>,
    pub assistant_event: u64,
    pub assistant_message: u64,
    pub token_usage_record: Option<u64>,
    pub token_count_event: Option<u64>,
    pub task_complete: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttributedTurn {
    pub turn_identity_sha256: String,
    pub declared_model: String,
    /// A fixed projection of recorded harness, prompt, reasoning-effort, and
    /// tool-policy fields. It is evidence for later context qualification, not
    /// by itself a released comparison stratum.
    pub recorded_configuration_sha256: String,
    pub usage_record_status: UsageRecordStatus,
    pub records: TurnRecordRefs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodexTaskAttributionEvidence {
    pub schema_version: u32,
    pub extractor_version: u32,
    pub source_profile: SourceProfileEvidence,
    pub source_digest: String,
    pub record_count: u64,
    pub recognized_records: u64,
    pub session_identity_sha256: Option<String>,
    pub state: TaskAttributionState,
}

fn invalid() -> anyhow::Error {
    anyhow!("insights-task-attribution-invalid")
}

fn record_index(index: usize) -> Result<u64> {
    u64::try_from(index).map_err(|_| invalid())
}

fn hash_domain(domain: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn source_profile(profile: CodexTaskSourceProfile) -> SourceProfileEvidence {
    let (profile_id, upstream_revision, observed_workspace_version, qualification_scope) =
        match profile {
            CodexTaskSourceProfile::PinnedDirectWriterFixtureC4017a87 => (
                "codex-writer-c4017a87-direct-fixture-v1",
                "c4017a87aacc7558002b7cb510025e967c1d765e",
                "0.0.0",
                QualificationScope::WriterFixtureOnly,
            ),
            CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords => (
                "codex-rust-v0.154.0-task-records-v1",
                "6b9826e3aa83b1a5947db50f4332cb9c65f1b340",
                "0.154.0",
                QualificationScope::ReleasedWriterTaskRecords,
            ),
        };
    SourceProfileEvidence {
        profile,
        profile_id: profile_id.into(),
        upstream_revision: upstream_revision.into(),
        observed_workspace_version: observed_workspace_version.into(),
        qualification_scope,
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
        })
}

impl CodexTaskAttributionEvidence {
    /// Validate cached evidence before a future store accepts it. A containing
    /// snapshot must separately compare `source_digest` with its source bytes.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.extractor_version != 1
            || self.source_profile != source_profile(self.source_profile.profile)
            || !valid_hash(&self.source_digest)
            || self.record_count == 0
            || self.record_count > MAX_SOURCE_BYTES as u64
            || self.recognized_records > self.record_count
            || self
                .session_identity_sha256
                .as_deref()
                .is_some_and(|identity| !valid_hash(identity))
        {
            return Err(invalid());
        }
        match &self.state {
            TaskAttributionState::Attributed { turns } => {
                if self.session_identity_sha256.is_none()
                    || self.recognized_records != self.record_count
                    || self.record_count > MAX_RECORDS as u64
                    || turns.is_empty()
                    || turns.len() > MAX_TURNS
                    || !turns
                        .windows(2)
                        .all(|pair| pair[0].records.task_complete < pair[1].records.task_started)
                    || turns
                        .iter()
                        .map(|turn| turn.turn_identity_sha256.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != turns.len()
                    || turns.iter().skip(1).any(|turn| {
                        turn.declared_model != turns[0].declared_model
                            || turn.recorded_configuration_sha256
                                != turns[0].recorded_configuration_sha256
                    })
                    || !has_full_context(&turns[0].records)
                    || turns
                        .iter()
                        .any(|turn| !valid_turn(turn, self.record_count))
                    || turns
                        .iter()
                        .flat_map(|turn| turn_record_indexes(&turn.records))
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != usize::try_from(self.record_count - 1).map_err(|_| invalid())?
                {
                    return Err(invalid());
                }
            }
            TaskAttributionState::Unavailable { record_index, .. } => {
                if record_index.is_some_and(|index| index == 0 || index > self.record_count) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

fn valid_turn(turn: &AttributedTurn, record_count: u64) -> bool {
    let refs = &turn.records;
    let full_context = [
        refs.permissions_context,
        refs.environment_context,
        refs.world_state,
    ];
    let context_shape =
        full_context.iter().all(Option::is_some) || full_context.iter().all(Option::is_none);
    let ordered = turn_record_indexes(refs);
    let usage_shape = match turn.usage_record_status {
        UsageRecordStatus::Valid => refs.token_usage_record.is_some(),
        UsageRecordStatus::Missing => refs.token_usage_record.is_none(),
        UsageRecordStatus::Invalid => {
            refs.token_usage_record.is_some()
                || refs.token_count_event.is_some()
                || !refs.intermediate_usage_records.is_empty()
        }
    };
    context_shape
        && usage_shape
        && refs
            .intermediate_usage_records
            .iter()
            .all(|index| refs.intermediate_records.binary_search(index).is_ok())
        && valid_hash(&turn.turn_identity_sha256)
        && valid_hash(&turn.recorded_configuration_sha256)
        && !turn.declared_model.is_empty()
        && turn.declared_model.len() <= 96
        && turn
            .declared_model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
        && ordered
            .iter()
            .all(|index| *index > 1 && *index <= record_count)
        && ordered.windows(2).all(|pair| pair[0] < pair[1])
}

fn has_full_context(refs: &TurnRecordRefs) -> bool {
    refs.permissions_context.is_some()
        && refs.environment_context.is_some()
        && refs.world_state.is_some()
}

fn turn_record_indexes(refs: &TurnRecordRefs) -> Vec<u64> {
    let mut ordered = vec![refs.task_started];
    ordered.extend(
        [
            refs.permissions_context,
            refs.environment_context,
            refs.world_state,
        ]
        .into_iter()
        .flatten(),
    );
    ordered.extend([refs.turn_context, refs.user_text, refs.user_event]);
    ordered.extend(refs.intermediate_records.iter().copied());
    ordered.extend([refs.assistant_event, refs.assistant_message]);
    ordered.extend(refs.token_usage_record);
    ordered.extend(refs.token_count_event);
    ordered.push(refs.task_complete);
    ordered
}

fn unavailable(
    profile: CodexTaskSourceProfile,
    source_digest: String,
    record_count: usize,
    recognized_records: usize,
    session_identity_sha256: Option<String>,
    reason: TaskAttributionUnavailableReason,
    record_index: Option<usize>,
) -> Result<CodexTaskAttributionEvidence> {
    let evidence = CodexTaskAttributionEvidence {
        schema_version: 1,
        extractor_version: 1,
        source_profile: source_profile(profile),
        source_digest,
        record_count: u64::try_from(record_count).map_err(|_| invalid())?,
        recognized_records: u64::try_from(recognized_records).map_err(|_| invalid())?,
        session_identity_sha256,
        state: TaskAttributionState::Unavailable {
            reason,
            record_index: record_index
                .map(u64::try_from)
                .transpose()
                .map_err(|_| invalid())?,
        },
    };
    evidence.validate()?;
    Ok(evidence)
}

#[derive(Serialize)]
struct ConfigurationProjection<'a> {
    originator: &'a str,
    observed_workspace_version: &'a str,
    prompt_sha256: String,
    approval_policy: &'a Value,
    approvals_reviewer: &'a Value,
    active_permission_profile_id: Option<&'a Value>,
    permission_profile: &'a Value,
    sandbox_policy: &'a Value,
    collaboration_mode: &'a Value,
    reasoning_effort: &'a Value,
    multi_agent_version: &'a Value,
}

struct SessionFacts {
    identity: String,
    identity_sha256: String,
    cwd: String,
    runtime_workspace_roots: Option<Value>,
    prompt_sha256: String,
    prompt_declared_model: Option<String>,
    originator: String,
    observed_workspace_version: String,
}

struct TurnBuilder {
    identity: String,
    started_at: u64,
    model: Option<String>,
    configuration_sha256: Option<String>,
    usage_status: UsageRecordStatus,
    pending_call_id: Option<String>,
    refs: TurnRecordRefs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    AfterStart,
    AfterPermissions,
    AfterEnvironment,
    AfterWorld,
    AfterContext,
    AfterUserText,
    AfterUserEvent,
    Tooling,
    AfterAssistantEvent,
    AfterAssistantMessage,
    AfterUsage,
    AfterTokenCount,
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value> {
    value.get(name).ok_or_else(invalid)
}

fn string_field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    field(value, name)?.as_str().ok_or_else(invalid)
}

fn u64_field(value: &Value, name: &str) -> Result<u64> {
    field(value, name)?.as_u64().ok_or_else(invalid)
}

fn content_kind(payload: &Value) -> Option<&str> {
    let kinds = payload
        .get("internal_chat_message_metadata_passthrough")?
        .get("content_item_kinds")?
        .as_array()?;
    (kinds.len() == 1).then(|| kinds[0].as_str()).flatten()
}

fn message_turn_id(payload: &Value) -> Option<&str> {
    payload
        .get("internal_chat_message_metadata_passthrough")?
        .get("turn_id")?
        .as_str()
}

fn text_blocks_match(payload: &Value, block_type: &str) -> bool {
    payload
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            !blocks.is_empty()
                && blocks.iter().all(|block| {
                    block.get("type").and_then(Value::as_str) == Some(block_type)
                        && block.get("text").is_some_and(Value::is_string)
                })
        })
}

fn parse_session(profile: CodexTaskSourceProfile, payload: &Value) -> Result<SessionFacts> {
    let profile_evidence = source_profile(profile);
    let identity = string_field(payload, "session_id")?;
    if !canonical_uuid(identity)
        || string_field(payload, "id")? != identity
        || string_field(payload, "cli_version")? != profile_evidence.observed_workspace_version
        || string_field(payload, "originator")? != "codex_cli_rs"
        || string_field(payload, "source")? != "exec"
        || string_field(payload, "history_mode")? != "legacy"
    {
        return Err(invalid());
    }
    let cwd = string_field(payload, "cwd")?;
    let roots = payload.get("runtime_workspace_roots").cloned();
    match profile {
        CodexTaskSourceProfile::PinnedDirectWriterFixtureC4017a87
            if roots.as_ref().is_none_or(|roots| !roots.is_array()) =>
        {
            return Err(invalid());
        }
        CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords if roots.is_some() => {
            return Err(invalid());
        }
        _ => {}
    }
    let prompt = field(payload, "base_instructions")?;
    let provenance = field(prompt, "provenance")?;
    let prompt_declared_model = match (profile, string_field(provenance, "type")?) {
        (_, "custom") => None,
        (CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords, "model") => {
            Some(string_field(provenance, "model")?.to_owned())
        }
        _ => return Err(invalid()),
    };
    let prompt = string_field(prompt, "text")?;
    Ok(SessionFacts {
        identity: identity.into(),
        identity_sha256: hash_domain("codex-session-identity-v1", identity),
        cwd: cwd.into(),
        runtime_workspace_roots: roots,
        prompt_sha256: hash_domain("codex-base-instructions-v1", prompt),
        prompt_declared_model,
        originator: "codex_cli_rs".into(),
        observed_workspace_version: profile_evidence.observed_workspace_version,
    })
}

fn record_kind(record: &Value) -> Result<(&str, &Value)> {
    let object = record.as_object().ok_or_else(invalid)?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let payload = object
        .get("payload")
        .filter(|v| v.is_object())
        .ok_or_else(invalid)?;
    Ok((kind, payload))
}

fn event_kind(payload: &Value) -> Option<&str> {
    payload.get("type").and_then(Value::as_str)
}

fn supported_record_semantics(
    profile: CodexTaskSourceProfile,
    kind: &str,
    payload: &Value,
) -> bool {
    match kind {
        "session_meta" | "world_state" | "turn_context" | "token_usage_record" => true,
        "event_msg" => {
            matches!(
                event_kind(payload),
                Some(
                    "task_started"
                        | "user_message"
                        | "agent_message"
                        | "token_count"
                        | "task_complete"
                )
            ) || (profile == CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords
                && event_kind(payload) == Some("agent_reasoning"))
        }
        "response_item" => {
            if profile == CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords
                && matches!(
                    payload.get("type").and_then(Value::as_str),
                    Some("reasoning" | "function_call" | "function_call_output")
                )
            {
                return true;
            }
            if payload.get("type").and_then(Value::as_str) != Some("message") {
                return false;
            }
            match payload.get("role").and_then(Value::as_str) {
                Some("developer") => content_kind(payload) == Some("permissions.instructions"),
                Some("user") => matches!(
                    content_kind(payload),
                    Some("environments.environment_context" | "user.text")
                ),
                Some("assistant") => {
                    message_turn_id(payload).is_some()
                        && content_kind(payload) == Some("unknown")
                        && text_blocks_match(payload, "output_text")
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn config_digest(session: &SessionFacts, context: &Value) -> Result<String> {
    let collaboration = field(context, "collaboration_mode")?;
    let settings = field(collaboration, "settings")?;
    let projection = ConfigurationProjection {
        originator: &session.originator,
        observed_workspace_version: &session.observed_workspace_version,
        prompt_sha256: session.prompt_sha256.clone(),
        approval_policy: field(context, "approval_policy")?,
        approvals_reviewer: field(context, "approvals_reviewer")?,
        active_permission_profile_id: context
            .get("active_permission_profile")
            .and_then(|profile| profile.get("id")),
        permission_profile: field(context, "permission_profile")?,
        sandbox_policy: field(context, "sandbox_policy")?,
        collaboration_mode: field(collaboration, "mode")?,
        reasoning_effort: field(settings, "reasoning_effort")?,
        multi_agent_version: field(context, "multi_agent_version")?,
    };
    let bytes = serde_json::to_vec(&projection).map_err(|_| invalid())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn world_models_agree(world: &Value, model: &str) -> bool {
    world.get("state").is_some_and(|state| {
        state.get("model").and_then(Value::as_str) == Some(model)
            && state
                .get("collaboration_mode")
                .and_then(|value| value.get("model"))
                .and_then(Value::as_str)
                == Some(model)
    })
}

fn world_context_agrees(session: &SessionFacts, world: &Value, context: &Value) -> bool {
    let state = match world.get("state") {
        Some(state) => state,
        None => return false,
    };
    let environment = state
        .get("environments")
        .and_then(|v| v.get("environments"))
        .and_then(|v| v.get("local"));
    environment
        .and_then(|v| v.get("cwd"))
        .and_then(Value::as_str)
        == Some(session.cwd.as_str())
        && state
            .get("collaboration_mode")
            .and_then(|value| value.get("mode"))
            .and_then(Value::as_str)
            == context
                .get("collaboration_mode")
                .and_then(|value| value.get("mode"))
                .and_then(Value::as_str)
        && context.get("cwd").and_then(Value::as_str) == Some(session.cwd.as_str())
        && session
            .runtime_workspace_roots
            .as_ref()
            .is_none_or(|roots| context.get("workspace_roots") == Some(roots))
        && state
            .get("environments")
            .and_then(|v| v.get("current_date"))
            == context.get("current_date")
        && state.get("environments").and_then(|v| v.get("timezone")) == context.get("timezone")
        && state.get("realtime").and_then(|value| value.get("active"))
            == context.get("realtime_active")
}

fn usage_agrees(payload: &Value, session: &SessionFacts, turn: &TurnBuilder) -> bool {
    let ids_agree = payload.get("session_id").and_then(Value::as_str)
        == Some(session.identity.as_str())
        && payload.get("thread_id").and_then(Value::as_str) == Some(session.identity.as_str())
        && payload.get("turn_id").and_then(Value::as_str) == Some(turn.identity.as_str())
        && payload.get("root_turn_id").and_then(Value::as_str) == Some(turn.identity.as_str());
    let counters_valid = ["usage", "turn_token_usage", "thread_token_usage"]
        .iter()
        .all(|name| payload.get(*name).is_some_and(usage_counters_valid));
    ids_agree && counters_valid
}

fn usage_counters_valid(usage: &Value) -> bool {
    let component_total = [
        "cache_write_input_tokens",
        "cached_input_tokens",
        "input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
    ]
    .iter()
    .try_fold(0u64, |sum, counter| {
        sum.checked_add(usage.get(*counter)?.as_u64()?)
    });
    component_total.is_some()
        && usage
            .get("total_tokens")
            .is_some_and(|value| value.as_u64().is_some())
}

fn token_count_valid(payload: &Value) -> bool {
    payload.get("info").is_some_and(|info| {
        ["last_token_usage", "total_token_usage"]
            .iter()
            .all(|name| info.get(*name).is_some_and(usage_counters_valid))
    })
}

/// Classify tasks only under the pinned writer-fixture profile.
///
/// Malformed JSON/envelopes fail with a fixed error. Structurally valid but
/// unsupported or ambiguous records return typed unavailable evidence; they
/// are never ignored. A malformed optional usage record is retained as
/// `Invalid` usage while the task boundary remains attributable.
pub fn classify_codex_task_attribution(
    profile: CodexTaskSourceProfile,
    bytes: &[u8],
) -> Result<CodexTaskAttributionEvidence> {
    if bytes.len() > MAX_SOURCE_BYTES {
        bail!("insights-task-attribution-source-too-large");
    }
    let source_digest = format!("{:x}", Sha256::digest(bytes));
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let record_count = text.lines().count();
    if record_count > MAX_RECORDS {
        return unavailable(
            profile,
            source_digest,
            record_count,
            0,
            None,
            TaskAttributionUnavailableReason::RecordLimitExceeded,
            Some(MAX_RECORDS + 1),
        );
    }
    let mut records = Vec::new();
    for (offset, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(invalid());
        }
        let value = serde_json::from_str(line).map_err(|_| invalid())?;
        records.push((offset + 1, value));
    }
    let Some((first_index, first)) = records.first() else {
        return Err(invalid());
    };
    let (first_kind, first_payload) = record_kind(first)?;
    if *first_index != 1 || first_kind != "session_meta" {
        return unavailable(
            profile,
            source_digest,
            record_count,
            0,
            None,
            TaskAttributionUnavailableReason::InvalidRecordOrder,
            Some(*first_index),
        );
    }
    if let (Some(id), Some(session_id)) = (
        first_payload.get("id").and_then(Value::as_str),
        first_payload.get("session_id").and_then(Value::as_str),
    ) && id != session_id
    {
        return unavailable(
            profile,
            source_digest,
            record_count,
            1,
            None,
            TaskAttributionUnavailableReason::IdentityConflict,
            Some(*first_index),
        );
    }
    if ["parent_thread_id", "forked_from_id"].iter().any(|field| {
        first_payload
            .get(*field)
            .is_some_and(|value| !value.is_null())
    }) {
        return unavailable(
            profile,
            source_digest,
            record_count,
            1,
            None,
            TaskAttributionUnavailableReason::ForkOrDelegation,
            Some(*first_index),
        );
    }
    let session = match parse_session(profile, first_payload) {
        Ok(session) => session,
        Err(_) => {
            return unavailable(
                profile,
                source_digest,
                record_count,
                1,
                None,
                TaskAttributionUnavailableReason::SourceProfileMismatch,
                Some(*first_index),
            );
        }
    };
    let mut recognized = 1usize;
    let mut stage = Stage::Idle;
    let mut turn: Option<TurnBuilder> = None;
    let mut turns: Vec<AttributedTurn> = Vec::new();
    let mut source_model: Option<String> = None;
    let mut source_config: Option<String> = None;
    let mut pending_world: Option<Value> = None;

    macro_rules! unavailable_here {
        ($reason:expr, $index:expr) => {
            return unavailable(
                profile,
                source_digest,
                record_count,
                recognized,
                Some(session.identity_sha256.clone()),
                $reason,
                Some($index),
            )
        };
    }

    for (index, record) in records.iter().skip(1) {
        let (kind, payload) = record_kind(record)?;
        let event = event_kind(payload);
        let response_type = payload.get("type").and_then(Value::as_str);
        match stage {
            Stage::Idle if kind == "event_msg" && event == Some("task_started") => {
                if turns.len() == MAX_TURNS {
                    unavailable_here!(TaskAttributionUnavailableReason::TurnLimitExceeded, *index);
                }
                let identity = string_field(payload, "turn_id")?;
                let identity_sha256 = hash_domain("codex-task-identity-v1", identity);
                if turns
                    .iter()
                    .any(|turn| turn.turn_identity_sha256 == identity_sha256)
                {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                let root_identity_valid = match profile {
                    CodexTaskSourceProfile::PinnedDirectWriterFixtureC4017a87 => {
                        string_field(payload, "root_turn_id")? == identity
                    }
                    CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords => {
                        payload.get("root_turn_id").is_none()
                    }
                };
                if !root_identity_valid
                    || string_field(payload, "collaboration_mode_kind")? != "default"
                {
                    unavailable_here!(TaskAttributionUnavailableReason::ForkOrDelegation, *index);
                }
                turn = Some(TurnBuilder {
                    identity: identity.into(),
                    started_at: u64_field(payload, "started_at")?,
                    model: None,
                    configuration_sha256: None,
                    usage_status: UsageRecordStatus::Missing,
                    pending_call_id: None,
                    refs: TurnRecordRefs {
                        task_started: record_index(*index)?,
                        permissions_context: None,
                        environment_context: None,
                        world_state: None,
                        turn_context: 0,
                        user_text: 0,
                        user_event: 0,
                        intermediate_records: Vec::new(),
                        intermediate_usage_records: Vec::new(),
                        assistant_event: 0,
                        assistant_message: 0,
                        token_usage_record: None,
                        token_count_event: None,
                        task_complete: 0,
                    },
                });
                stage = Stage::AfterStart;
            }
            Stage::AfterStart
                if kind == "response_item"
                    && response_type == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("developer")
                    && content_kind(payload) == Some("permissions.instructions")
                    && text_blocks_match(payload, "input_text") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if message_turn_id(payload) != Some(active.identity.as_str()) {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                active.refs.permissions_context = Some(record_index(*index)?);
                stage = Stage::AfterPermissions;
            }
            Stage::AfterPermissions
                if kind == "response_item"
                    && response_type == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("user")
                    && content_kind(payload) == Some("environments.environment_context")
                    && text_blocks_match(payload, "input_text") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if message_turn_id(payload) != Some(active.identity.as_str()) {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                active.refs.environment_context = Some(record_index(*index)?);
                stage = Stage::AfterEnvironment;
            }
            Stage::AfterEnvironment if kind == "world_state" => {
                pending_world = Some(payload.clone());
                turn.as_mut().ok_or_else(invalid)?.refs.world_state = Some(record_index(*index)?);
                stage = Stage::AfterWorld;
            }
            Stage::AfterStart | Stage::AfterWorld if kind == "turn_context" => {
                if stage == Stage::AfterStart && turns.is_empty() {
                    unavailable_here!(
                        TaskAttributionUnavailableReason::MissingRequiredRecord,
                        *index
                    );
                }
                let active = turn.as_mut().ok_or_else(invalid)?;
                let identity = string_field(payload, "turn_id")?;
                if identity != active.identity || string_field(payload, "root_turn_id")? != identity
                {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                if payload
                    .get("collaboration_mode")
                    .and_then(|v| v.get("mode"))
                    .and_then(Value::as_str)
                    != Some("default")
                {
                    unavailable_here!(TaskAttributionUnavailableReason::ForkOrDelegation, *index);
                }
                let model = string_field(payload, "model")?;
                if session
                    .prompt_declared_model
                    .as_deref()
                    .is_some_and(|declared| declared != model)
                {
                    unavailable_here!(TaskAttributionUnavailableReason::ModelConflict, *index);
                }
                if payload
                    .get("collaboration_mode")
                    .and_then(|v| v.get("settings"))
                    .and_then(|v| v.get("model"))
                    .and_then(Value::as_str)
                    != Some(model)
                {
                    unavailable_here!(TaskAttributionUnavailableReason::ModelConflict, *index);
                }
                if let Some(world) = pending_world.take() {
                    if !world_models_agree(&world, model) {
                        unavailable_here!(TaskAttributionUnavailableReason::ModelConflict, *index);
                    }
                    if !world_context_agrees(&session, &world, payload) {
                        unavailable_here!(
                            TaskAttributionUnavailableReason::ContextConflict,
                            *index
                        );
                    }
                }
                if payload.get("cwd").and_then(Value::as_str) != Some(session.cwd.as_str())
                    || session
                        .runtime_workspace_roots
                        .as_ref()
                        .is_some_and(|roots| payload.get("workspace_roots") != Some(roots))
                {
                    unavailable_here!(TaskAttributionUnavailableReason::ContextConflict, *index);
                }
                if source_model.as_deref().is_some_and(|known| known != model) {
                    unavailable_here!(TaskAttributionUnavailableReason::ModelConflict, *index);
                }
                source_model.get_or_insert_with(|| model.into());
                let configuration = config_digest(&session, payload)?;
                if source_config
                    .as_deref()
                    .is_some_and(|known| known != configuration)
                {
                    unavailable_here!(
                        TaskAttributionUnavailableReason::ConfigurationConflict,
                        *index
                    );
                }
                source_config.get_or_insert_with(|| configuration.clone());
                active.model = Some(model.into());
                active.configuration_sha256 = Some(configuration);
                active.refs.turn_context = record_index(*index)?;
                stage = Stage::AfterContext;
            }
            Stage::AfterContext
                if kind == "response_item"
                    && response_type == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("user")
                    && content_kind(payload) == Some("user.text")
                    && text_blocks_match(payload, "input_text") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if message_turn_id(payload) != Some(active.identity.as_str()) {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                active.refs.user_text = record_index(*index)?;
                stage = Stage::AfterUserText;
            }
            Stage::AfterUserText
                if kind == "event_msg"
                    && event == Some("user_message")
                    && payload.get("message").is_some_and(Value::is_string) =>
            {
                turn.as_mut().ok_or_else(invalid)?.refs.user_event = record_index(*index)?;
                stage = Stage::AfterUserEvent;
            }
            Stage::AfterUserEvent
                if profile == CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords
                    && kind == "event_msg"
                    && event == Some("agent_reasoning")
                    && payload.get("text").is_some_and(Value::is_string) =>
            {
                turn.as_mut()
                    .ok_or_else(invalid)?
                    .refs
                    .intermediate_records
                    .push(record_index(*index)?);
                stage = Stage::Tooling;
            }
            Stage::Tooling
                if kind == "response_item"
                    && response_type == Some("reasoning")
                    && payload.get("summary").is_some_and(Value::is_array)
                    && payload
                        .get("encrypted_content")
                        .is_some_and(Value::is_string) =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if active.pending_call_id.is_some()
                    || message_turn_id(payload) != Some(active.identity.as_str())
                {
                    unavailable_here!(TaskAttributionUnavailableReason::InvalidRecordOrder, *index);
                }
                active.refs.intermediate_records.push(record_index(*index)?);
            }
            Stage::Tooling
                if kind == "response_item"
                    && response_type == Some("function_call")
                    && payload.get("name").and_then(Value::as_str) == Some("exec_command") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                let call_id = string_field(payload, "call_id")?;
                let arguments = string_field(payload, "arguments")?;
                let arguments: Value = serde_json::from_str(arguments).map_err(|_| invalid())?;
                if active.pending_call_id.is_some()
                    || message_turn_id(payload) != Some(active.identity.as_str())
                    || !arguments
                        .get("cmd")
                        .is_some_and(|command| command.is_string())
                {
                    unavailable_here!(TaskAttributionUnavailableReason::InvalidRecordOrder, *index);
                }
                active.pending_call_id = Some(call_id.into());
                active.refs.intermediate_records.push(record_index(*index)?);
            }
            Stage::Tooling if kind == "token_usage_record" => {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if !usage_agrees(payload, &session, active) {
                    active.usage_status = UsageRecordStatus::Invalid;
                }
                active.refs.intermediate_records.push(record_index(*index)?);
                active
                    .refs
                    .intermediate_usage_records
                    .push(record_index(*index)?);
            }
            Stage::Tooling
                if kind == "response_item" && response_type == Some("function_call_output") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if active.pending_call_id.as_deref() != Some(string_field(payload, "call_id")?)
                    || message_turn_id(payload) != Some(active.identity.as_str())
                    || !payload.get("output").is_some_and(Value::is_string)
                {
                    unavailable_here!(TaskAttributionUnavailableReason::InvalidRecordOrder, *index);
                }
                active.pending_call_id = None;
                active.refs.intermediate_records.push(record_index(*index)?);
            }
            Stage::Tooling if kind == "event_msg" && event == Some("token_count") => {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if !token_count_valid(payload) {
                    active.usage_status = UsageRecordStatus::Invalid;
                }
                active.refs.intermediate_records.push(record_index(*index)?);
                active
                    .refs
                    .intermediate_usage_records
                    .push(record_index(*index)?);
            }
            Stage::AfterUserEvent | Stage::Tooling
                if kind == "event_msg"
                    && event == Some("agent_message")
                    && payload.get("message").is_some_and(Value::is_string) =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if active.pending_call_id.is_some() {
                    unavailable_here!(TaskAttributionUnavailableReason::InvalidRecordOrder, *index);
                }
                active.refs.assistant_event = record_index(*index)?;
                stage = Stage::AfterAssistantEvent;
            }
            Stage::AfterAssistantEvent
                if kind == "response_item"
                    && response_type == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("assistant")
                    && content_kind(payload) == Some("unknown")
                    && text_blocks_match(payload, "output_text") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if message_turn_id(payload) != Some(active.identity.as_str()) {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                active.refs.assistant_message = record_index(*index)?;
                stage = Stage::AfterAssistantMessage;
            }
            Stage::AfterAssistantMessage if kind == "token_usage_record" => {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if !usage_agrees(payload, &session, active) {
                    active.usage_status = UsageRecordStatus::Invalid;
                } else if active.usage_status != UsageRecordStatus::Invalid {
                    active.usage_status = UsageRecordStatus::Valid;
                }
                active.refs.token_usage_record = Some(record_index(*index)?);
                stage = Stage::AfterUsage;
            }
            Stage::AfterAssistantMessage | Stage::AfterUsage
                if kind == "event_msg" && event == Some("token_count") =>
            {
                let active = turn.as_mut().ok_or_else(invalid)?;
                if !token_count_valid(payload) {
                    active.usage_status = UsageRecordStatus::Invalid;
                }
                active.refs.token_count_event = Some(record_index(*index)?);
                stage = Stage::AfterTokenCount;
            }
            Stage::AfterAssistantMessage | Stage::AfterUsage | Stage::AfterTokenCount
                if kind == "event_msg" && event == Some("task_complete") =>
            {
                let mut active = turn.take().ok_or_else(invalid)?;
                if string_field(payload, "turn_id")? != active.identity
                    || u64_field(payload, "started_at")? != active.started_at
                    || u64_field(payload, "completed_at")? < active.started_at
                {
                    unavailable_here!(TaskAttributionUnavailableReason::IdentityConflict, *index);
                }
                active.refs.task_complete = record_index(*index)?;
                turns.push(AttributedTurn {
                    turn_identity_sha256: hash_domain("codex-task-identity-v1", &active.identity),
                    declared_model: active.model.ok_or_else(invalid)?,
                    recorded_configuration_sha256: active
                        .configuration_sha256
                        .ok_or_else(invalid)?,
                    usage_record_status: active.usage_status,
                    records: active.refs,
                });
                stage = Stage::Idle;
            }
            _ => {
                unavailable_here!(
                    if supported_record_semantics(profile, kind, payload) {
                        TaskAttributionUnavailableReason::InvalidRecordOrder
                    } else {
                        TaskAttributionUnavailableReason::UnsupportedRecord
                    },
                    *index
                );
            }
        }
        recognized = recognized.checked_add(1).ok_or_else(invalid)?;
    }
    if stage != Stage::Idle || turn.is_some() || turns.is_empty() {
        return unavailable(
            profile,
            source_digest,
            record_count,
            recognized,
            Some(session.identity_sha256.clone()),
            TaskAttributionUnavailableReason::MissingRequiredRecord,
            records.last().map(|(index, _)| *index),
        );
    }
    let evidence = CodexTaskAttributionEvidence {
        schema_version: 1,
        extractor_version: 1,
        source_profile: source_profile(profile),
        source_digest,
        record_count: u64::try_from(record_count).map_err(|_| invalid())?,
        recognized_records: u64::try_from(recognized).map_err(|_| invalid())?,
        session_identity_sha256: Some(session.identity_sha256),
        state: TaskAttributionState::Attributed { turns },
    };
    evidence.validate()?;
    Ok(evidence)
}

/// Preserve generic Codex import compatibility when a valid transcript falls
/// outside the qualified source profile. The unavailable evidence remains
/// bound to the exact bytes and cannot be promoted by caller assertion.
pub fn classify_codex_task_attribution_for_import(
    profile: CodexTaskSourceProfile,
    bytes: &[u8],
) -> Result<CodexTaskAttributionEvidence> {
    match classify_codex_task_attribution(profile, bytes) {
        Ok(evidence) => Ok(evidence),
        Err(_) if !bytes.is_empty() && bytes.len() <= MAX_SOURCE_BYTES => unavailable(
            profile,
            format!("{:x}", Sha256::digest(bytes)),
            std::str::from_utf8(bytes)
                .map_err(|_| invalid())?
                .lines()
                .count(),
            0,
            None,
            TaskAttributionUnavailableReason::SourceProfileMismatch,
            None,
        ),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALPHA: &[u8] =
        include_bytes!("../../fixtures/insights/codex-task-attribution/codex-alpha-direct.jsonl");
    const BETA: &[u8] =
        include_bytes!("../../fixtures/insights/codex-task-attribution/codex-beta-direct.jsonl");
    const RELEASE_ALPHA: &[u8] = include_bytes!(
        "../../fixtures/insights/codex-task-attribution/codex-release-0.154.0-alpha-direct.jsonl"
    );
    const RELEASE_BETA: &[u8] = include_bytes!(
        "../../fixtures/insights/codex-task-attribution/codex-release-0.154.0-beta-direct.jsonl"
    );
    const RELEASE_TOOL: &[u8] = include_bytes!(
        "../../fixtures/insights/codex-task-attribution/codex-release-0.154.0-tool-reasoning.jsonl"
    );
    const RELEASE_DEFAULT: &[u8] = include_bytes!(
        "../../fixtures/insights/codex-task-attribution/codex-release-0.154.0-default-instructions.jsonl"
    );
    const PROFILE: CodexTaskSourceProfile =
        CodexTaskSourceProfile::PinnedDirectWriterFixtureC4017a87;

    fn decode(bytes: &[u8]) -> Vec<Value> {
        std::str::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn encode(records: &[Value]) -> Vec<u8> {
        let mut encoded = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        encoded.push(b'\n');
        encoded
    }

    fn unavailable_reason(bytes: &[u8]) -> TaskAttributionUnavailableReason {
        unavailable_reason_for(PROFILE, bytes)
    }

    fn unavailable_reason_for(
        profile: CodexTaskSourceProfile,
        bytes: &[u8],
    ) -> TaskAttributionUnavailableReason {
        match classify_codex_task_attribution(profile, bytes)
            .unwrap()
            .state
        {
            TaskAttributionState::Unavailable { reason, .. } => reason,
            TaskAttributionState::Attributed { .. } => panic!("fixture unexpectedly attributed"),
        }
    }

    fn find_record(records: &[Value], top_level: &str, payload_type: Option<&str>) -> usize {
        records
            .iter()
            .position(|record| {
                record["type"] == top_level
                    && payload_type.is_none_or(|kind| record["payload"]["type"] == kind)
            })
            .unwrap()
    }

    fn replace_string(value: &mut Value, from: &str, to: &str) {
        match value {
            Value::String(text) if text == from => *text = to.to_owned(),
            Value::Array(values) => {
                for value in values {
                    replace_string(value, from, to);
                }
            }
            Value::Object(object) => {
                for value in object.values_mut() {
                    replace_string(value, from, to);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn writer_fixtures_form_two_distinct_fixture_only_cohorts() {
        let alpha = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        let beta = classify_codex_task_attribution(PROFILE, BETA).unwrap();
        assert_eq!(
            alpha.source_digest,
            "55ffa88eda7b84ffbb38297158df61af5e7dc958eba685192fec380483ab2962"
        );
        assert_eq!(
            beta.source_digest,
            "05ae5ba57052466c1ffd814540459e479cc1f43ca132b3a5ebf4913a21c17e93"
        );
        assert_eq!(
            alpha.source_profile.qualification_scope,
            QualificationScope::WriterFixtureOnly
        );
        assert_eq!(alpha.source_profile.observed_workspace_version, "0.0.0");
        let TaskAttributionState::Attributed { turns: alpha_turns } = alpha.state else {
            panic!("alpha fixture was not attributed")
        };
        let TaskAttributionState::Attributed { turns: beta_turns } = beta.state else {
            panic!("beta fixture was not attributed")
        };
        assert_eq!(alpha_turns.len(), 1);
        assert_eq!(beta_turns.len(), 2);
        assert!(
            alpha_turns
                .iter()
                .all(|turn| turn.declared_model == "model-alpha")
        );
        assert!(
            beta_turns
                .iter()
                .all(|turn| turn.declared_model == "model-beta")
        );
        assert_ne!(alpha.session_identity_sha256, beta.session_identity_sha256);
        assert_ne!(
            alpha_turns[0].turn_identity_sha256,
            beta_turns[0].turn_identity_sha256
        );
        assert_eq!(
            alpha_turns[0].recorded_configuration_sha256,
            beta_turns[0].recorded_configuration_sha256,
            "model, identity, date, and path evidence must not become configuration strata"
        );
        assert!(alpha_turns.iter().chain(&beta_turns).all(|turn| {
            turn.usage_record_status == UsageRecordStatus::Valid
                && turn.records.task_started < turn.records.task_complete
        }));
    }

    #[test]
    fn released_writer_profile_attributes_only_its_exact_direct_message_shape() {
        let profile = CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords;
        let alpha = classify_codex_task_attribution(profile, RELEASE_ALPHA).unwrap();
        let beta = classify_codex_task_attribution(profile, RELEASE_BETA).unwrap();
        assert_eq!(
            alpha.source_digest,
            "a254d4965ec6250f87386ff390ab9a273dd60ad0e9b140091027affa126e0d9a"
        );
        assert_eq!(
            beta.source_digest,
            "3fdbb19252144df71527ccdf9f142a9ae9e0c91ce9a5485dea139761a3b78fef"
        );
        assert_eq!(
            alpha.source_profile.qualification_scope,
            QualificationScope::ReleasedWriterTaskRecords
        );
        assert_eq!(alpha.source_profile.observed_workspace_version, "0.154.0");
        let TaskAttributionState::Attributed { turns: alpha_turns } = alpha.state else {
            panic!("released alpha fixture was not attributed")
        };
        let TaskAttributionState::Attributed { turns: beta_turns } = beta.state else {
            panic!("released beta fixture was not attributed")
        };
        assert_eq!(alpha_turns.len(), 1);
        assert_eq!(beta_turns.len(), 2);
        assert!(
            alpha_turns
                .iter()
                .all(|turn| turn.declared_model == "model-alpha")
        );
        assert!(
            beta_turns
                .iter()
                .all(|turn| turn.declared_model == "model-beta")
        );

        let mut unsupported = decode(RELEASE_ALPHA);
        unsupported.insert(
            8,
            json!({"type":"response_item","payload":{"type":"reasoning","summary":[]}}),
        );
        let evidence = classify_codex_task_attribution(profile, &encode(&unsupported)).unwrap();
        assert!(matches!(
            evidence.state,
            TaskAttributionState::Unavailable {
                reason: TaskAttributionUnavailableReason::InvalidRecordOrder,
                ..
            }
        ));
    }

    #[test]
    fn generic_import_keeps_profile_mismatches_as_typed_unavailable_evidence() {
        let mut bytes = RELEASE_ALPHA.to_vec();
        bytes.push(b'\n');
        let profile = CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords;
        assert!(classify_codex_task_attribution(profile, &bytes).is_err());
        let evidence = classify_codex_task_attribution_for_import(profile, &bytes).unwrap();
        assert!(matches!(
            evidence.state,
            TaskAttributionState::Unavailable {
                reason: TaskAttributionUnavailableReason::SourceProfileMismatch,
                ..
            }
        ));
    }

    #[test]
    fn released_writer_profile_qualifies_paired_reasoning_and_exec_records() {
        let profile = CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords;
        let evidence = classify_codex_task_attribution(profile, RELEASE_TOOL).unwrap();
        assert_eq!(
            evidence.source_digest,
            "b0bbcd2ce3b35a9b96f89b3c9ead28888d61c0b98fe77dd9a1b386518374a9a5"
        );
        let TaskAttributionState::Attributed { turns } = evidence.state else {
            panic!("released tool fixture was not attributed")
        };
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].declared_model, "model-tool");
        assert_eq!(
            turns[0].records.intermediate_records,
            vec![9, 10, 11, 12, 13, 14]
        );

        let mut mismatched = decode(RELEASE_TOOL);
        mismatched[12]["payload"]["call_id"] = serde_json::json!("mismatched-call");
        let evidence = classify_codex_task_attribution(profile, &encode(&mismatched)).unwrap();
        assert!(matches!(
            evidence.state,
            TaskAttributionState::Unavailable {
                reason: TaskAttributionUnavailableReason::InvalidRecordOrder,
                ..
            }
        ));

        let mut invalid_usage = decode(RELEASE_TOOL);
        invalid_usage[11]["payload"]["turn_token_usage"]["input_tokens"] =
            serde_json::json!(u64::MAX);
        invalid_usage[11]["payload"]["turn_token_usage"]["output_tokens"] = serde_json::json!(1u64);
        let evidence = classify_codex_task_attribution(profile, &encode(&invalid_usage)).unwrap();
        let TaskAttributionState::Attributed { turns } = evidence.state else {
            panic!("invalid optional usage suppressed structural attribution")
        };
        assert_eq!(turns[0].usage_record_status, UsageRecordStatus::Invalid);

        let mut no_final_usage = invalid_usage;
        no_final_usage.remove(17);
        no_final_usage.remove(16);
        let evidence = classify_codex_task_attribution(profile, &encode(&no_final_usage)).unwrap();
        let TaskAttributionState::Attributed { turns } = evidence.state else {
            panic!("intermediate invalid usage required an optional final usage pair")
        };
        assert_eq!(turns[0].usage_record_status, UsageRecordStatus::Invalid);
        assert!(turns[0].records.token_usage_record.is_none());
        assert!(turns[0].records.token_count_event.is_none());
    }

    #[test]
    fn released_default_model_instructions_require_the_same_declared_model() {
        let profile = CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords;
        let evidence = classify_codex_task_attribution(profile, RELEASE_DEFAULT).unwrap();
        let TaskAttributionState::Attributed { turns } = evidence.state else {
            panic!("released default-instruction fixture was not attributed")
        };
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].declared_model, "model-default");

        let mut mismatch = decode(RELEASE_DEFAULT);
        mismatch[0]["payload"]["base_instructions"]["provenance"]["model"] =
            serde_json::json!("different-model");
        assert_eq!(
            unavailable_reason_for(profile, &encode(&mismatch)),
            TaskAttributionUnavailableReason::ModelConflict
        );

        let mut unsupported = decode(RELEASE_DEFAULT);
        unsupported[0]["payload"]["base_instructions"]["provenance"] =
            serde_json::json!({"type":"bundled"});
        assert_eq!(
            unavailable_reason_for(profile, &encode(&unsupported)),
            TaskAttributionUnavailableReason::SourceProfileMismatch
        );
    }

    #[test]
    fn malformed_unknown_and_unqualified_records_never_disappear() {
        assert_eq!(
            classify_codex_task_attribution(PROFILE, b"not json\n")
                .unwrap_err()
                .to_string(),
            "insights-task-attribution-invalid"
        );
        let mut records = decode(ALPHA);
        records.insert(
            7,
            json!({"type":"response_item","payload":{"type":"reasoning","summary":[]}}),
        );
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::UnsupportedRecord
        );
        records = decode(ALPHA);
        let task_start = find_record(&records, "event_msg", Some("task_started"));
        records.insert(
            task_start + 1,
            json!({"type":"response_item","payload":{"type":"function_call","call_id":"x"}}),
        );
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::UnsupportedRecord
        );
    }

    #[test]
    fn task_sequence_and_required_initial_context_are_exact() {
        let mut records = decode(ALPHA);
        let user = find_record(&records, "event_msg", Some("user_message"));
        records.swap(user, user + 1);
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::InvalidRecordOrder
        );

        records = decode(ALPHA);
        records.retain(|record| {
            record["type"] != "world_state"
                && record["payload"]["internal_chat_message_metadata_passthrough"]
                    ["content_item_kinds"]
                    != json!(["permissions.instructions"])
                && record["payload"]["internal_chat_message_metadata_passthrough"]
                    ["content_item_kinds"]
                    != json!(["environments.environment_context"])
        });
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::MissingRequiredRecord
        );

        records = decode(ALPHA);
        records.pop();
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::MissingRequiredRecord
        );
    }

    #[test]
    fn raw_identity_never_survives_and_every_cross_record_id_must_agree() {
        let mut records = decode(ALPHA);
        records[0]["payload"]["id"] = json!("different-session");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::IdentityConflict
        );

        records = decode(ALPHA);
        let complete = find_record(&records, "event_msg", Some("task_complete"));
        records[complete]["payload"]["turn_id"] = json!("different-turn");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::IdentityConflict
        );

        let evidence = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        let serialized = serde_json::to_string(&evidence).unwrap();
        for raw in [
            "00000000-0000-4000-8100-000000000002",
            "00000000-0000-4000-8100-000000000003",
            "/w",
            "synthetic-alpha-text",
        ] {
            assert!(!serialized.contains(raw));
        }
    }

    #[test]
    fn forks_delegation_and_model_switches_are_unavailable() {
        let mut records = decode(ALPHA);
        let start = find_record(&records, "event_msg", Some("task_started"));
        records[start]["payload"]["root_turn_id"] = json!("parent-turn");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ForkOrDelegation
        );

        records = decode(ALPHA);
        records[0]["payload"]["forked_from_id"] = json!("parent-session");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ForkOrDelegation
        );

        records = decode(ALPHA);
        records[start]["payload"]["collaboration_mode_kind"] = json!("plan");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ForkOrDelegation
        );

        records = decode(BETA);
        let contexts: Vec<_> = records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| (record["type"] == "turn_context").then_some(index))
            .collect();
        records[contexts[1]]["payload"]["model"] = json!("model-switched");
        records[contexts[1]]["payload"]["collaboration_mode"]["settings"]["model"] =
            json!("model-switched");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ModelConflict
        );

        records = decode(ALPHA);
        let world = find_record(&records, "world_state", None);
        records[world]["payload"]["state"]["model"] = json!("other-model");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ModelConflict
        );
    }

    #[test]
    fn context_switch_and_configuration_conflict_are_distinct() {
        let mut records = decode(BETA);
        let contexts: Vec<_> = records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| (record["type"] == "turn_context").then_some(index))
            .collect();
        records[contexts[1]]["payload"]["cwd"] = json!("/different-checkout");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ContextConflict
        );

        records = decode(BETA);
        records[contexts[1]]["payload"]["approval_policy"] = json!("never");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::ConfigurationConflict
        );
    }

    #[test]
    fn optional_usage_invalidity_and_overflow_do_not_suppress_task_attribution() {
        let mut records = decode(ALPHA);
        let usage = find_record(&records, "token_usage_record", None);
        records[usage]["payload"]["turn_token_usage"]["input_tokens"] = json!(u64::MAX);
        records[usage]["payload"]["turn_token_usage"]["output_tokens"] = json!(1u64);
        let evidence = classify_codex_task_attribution(PROFILE, &encode(&records)).unwrap();
        let TaskAttributionState::Attributed { turns } = evidence.state else {
            panic!("usage overflow suppressed task attribution")
        };
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].usage_record_status, UsageRecordStatus::Invalid);
        assert_eq!(turns[0].records.task_complete, 13);

        records = decode(ALPHA);
        records.remove(usage);
        let evidence = classify_codex_task_attribution(PROFILE, &encode(&records)).unwrap();
        let TaskAttributionState::Attributed { turns } = evidence.state else {
            panic!("missing optional usage suppressed task attribution")
        };
        assert_eq!(turns[0].usage_record_status, UsageRecordStatus::Missing);
    }

    #[test]
    fn source_build_version_cannot_be_used_as_a_released_profile() {
        let mut records = decode(ALPHA);
        records[0]["payload"]["cli_version"] = json!("0.154.0");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::SourceProfileMismatch
        );
    }

    #[test]
    fn resumed_task_identity_and_all_input_bounds_are_enforced() {
        let mut records = decode(BETA);
        let turn_ids = records
            .iter()
            .filter(|record| record["type"] == "turn_context")
            .map(|record| record["payload"]["turn_id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        for record in &mut records {
            replace_string(record, &turn_ids[1], &turn_ids[0]);
        }
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::IdentityConflict
        );

        let too_many_records = "{}\n".repeat(MAX_RECORDS + 1);
        assert_eq!(
            unavailable_reason(too_many_records.as_bytes()),
            TaskAttributionUnavailableReason::RecordLimitExceeded
        );
        assert_eq!(
            classify_codex_task_attribution(PROFILE, &vec![b'x'; MAX_SOURCE_BYTES + 1])
                .unwrap_err()
                .to_string(),
            "insights-task-attribution-source-too-large"
        );

        records = decode(BETA);
        let second_start = records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                record["type"] == "event_msg" && record["payload"]["type"] == "task_started"
            })
            .nth(1)
            .unwrap()
            .0;
        let template = records[second_start..].to_vec();
        for extra in 0..(MAX_TURNS - 1) {
            let mut task = template.clone();
            let replacement = format!("bounded-extra-turn-{extra}");
            for record in &mut task {
                replace_string(record, &turn_ids[1], &replacement);
            }
            records.extend(task);
        }
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::TurnLimitExceeded
        );
    }

    #[test]
    fn cache_validation_rejects_forged_counts_hashes_and_coordinates() {
        let mut evidence = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        evidence.recognized_records = u64::MAX;
        assert_eq!(
            evidence.validate().unwrap_err().to_string(),
            invalid().to_string()
        );

        evidence = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        let TaskAttributionState::Attributed { turns } = &mut evidence.state else {
            unreachable!()
        };
        turns[0].records.task_complete = u64::MAX;
        assert!(evidence.validate().is_err());

        evidence = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        let TaskAttributionState::Attributed { turns } = &mut evidence.state else {
            unreachable!()
        };
        turns[0].records.task_started = 1;
        assert!(evidence.validate().is_err());

        evidence = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        let TaskAttributionState::Attributed { turns } = &mut evidence.state else {
            unreachable!()
        };
        turns[0].usage_record_status = UsageRecordStatus::Valid;
        turns[0].records.token_usage_record = None;
        assert!(evidence.validate().is_err());

        evidence = classify_codex_task_attribution(PROFILE, BETA).unwrap();
        let TaskAttributionState::Attributed { turns } = &mut evidence.state else {
            unreachable!()
        };
        turns[1].turn_identity_sha256 = turns[0].turn_identity_sha256.clone();
        assert!(evidence.validate().is_err());

        evidence = classify_codex_task_attribution(PROFILE, BETA).unwrap();
        let TaskAttributionState::Attributed { turns } = &mut evidence.state else {
            unreachable!()
        };
        turns[1].declared_model = "different-model".into();
        assert!(evidence.validate().is_err());

        evidence = classify_codex_task_attribution(PROFILE, BETA).unwrap();
        let TaskAttributionState::Attributed { turns } = &mut evidence.state else {
            unreachable!()
        };
        turns[0].records.task_complete = turns[1].records.task_started;
        assert!(evidence.validate().is_err());

        evidence = classify_codex_task_attribution(PROFILE, ALPHA).unwrap();
        evidence.source_profile.qualification_scope = QualificationScope::WriterFixtureOnly;
        evidence.source_profile.profile_id = "released-by-caller".into();
        assert!(evidence.validate().is_err());
    }

    #[test]
    fn session_identity_must_be_a_canonical_uuid() {
        let mut records = decode(ALPHA);
        records[0]["payload"]["id"] = json!("not-a-uuid");
        records[0]["payload"]["session_id"] = json!("not-a-uuid");
        assert_eq!(
            unavailable_reason(&encode(&records)),
            TaskAttributionUnavailableReason::SourceProfileMismatch
        );
    }
}
