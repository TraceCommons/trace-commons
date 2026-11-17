//! Review-only inspection of public schema literals and dynamic JSON keys.
//! Catalog matches describe public syntax; they never authenticate a trace.
use super::*;

pub const REVIEW_KEY_CATALOG_VERSION: &str = "trace_commons.review_key_catalog.v1";
#[cfg(feature = "near-ai-privacy-filter")]
const MAX_KEY_FINDINGS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKeyReason {
    ClassifierFinding,
    ClassifierUnavailable,
    InvalidEvidence,
    DeterministicFinding,
    BudgetExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewKeyFinding {
    /// Domain-separated hash of typed, length-prefixed location components.
    pub field_hash: String,
    /// Fixed classifier vocabulary, never a backend-authored arbitrary label.
    pub category: String,
    pub confidence_millionths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub reason: ReviewKeyReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewKeyDiagnostics {
    pub catalog_version: String,
    pub schema_keys_recognized: u32,
    pub contextual_keys_examined: u32,
    pub findings: Vec<ReviewKeyFinding>,
    pub truncated: bool,
}
impl Default for ReviewKeyDiagnostics {
    fn default() -> Self {
        Self {
            catalog_version: REVIEW_KEY_CATALOG_VERSION.into(),
            schema_keys_recognized: 0,
            contextual_keys_examined: 0,
            findings: Vec::new(),
            truncated: false,
        }
    }
}

#[cfg(feature = "near-ai-privacy-filter")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    PayloadRoot,
    Arguments,
    PlanArray,
    PlanEntry,
    Other,
}

#[cfg(feature = "near-ai-privacy-filter")]
enum ValueKind {
    String,
    Boolean,
    Unsigned,
    Array,
    Choice(&'static [&'static str]),
}
#[cfg(feature = "near-ai-privacy-filter")]
impl ValueKind {
    fn matches(&self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Boolean => value.is_boolean(),
            Self::Unsigned => value.as_u64().is_some(),
            Self::Array => value.is_array(),
            Self::Choice(choices) => value.as_str().is_some_and(|v| choices.contains(&v)),
        }
    }
}
#[cfg(feature = "near-ai-privacy-filter")]
struct Property {
    position: Position,
    key: &'static str,
    kind: ValueKind,
}
#[cfg(feature = "near-ai-privacy-filter")]
struct PublicToolSchema {
    version: &'static str,
    aliases: &'static [&'static str],
    properties: &'static [Property],
}

// Reviewed public API spellings, not caller-selected schema declarations.
// A match exempts ONLY the literal property token, never its value/subtree.
#[cfg(feature = "near-ai-privacy-filter")]
const PUBLIC_TOOL_SCHEMAS: &[PublicToolSchema] = &[
    PublicToolSchema {
        version: "codex.spawn_agent.v1",
        aliases: &[
            "spawn_agent",
            "functions.spawn_agent",
            "collaboration.spawn_agent",
            "functions.collaboration.spawn_agent",
        ],
        properties: &[
            Property {
                position: Position::Arguments,
                key: "task_name",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "message",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "model",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "fork_turns",
                kind: ValueKind::Choice(&["all", "none"]),
            },
            Property {
                position: Position::Arguments,
                key: "reasoning_effort",
                kind: ValueKind::Choice(&["low", "medium", "high", "xhigh", "max", "ultra"]),
            },
        ],
    },
    PublicToolSchema {
        version: "codex.update_plan.v1",
        aliases: &["update_plan", "functions.update_plan"],
        properties: &[
            Property {
                position: Position::Arguments,
                key: "explanation",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "plan",
                kind: ValueKind::Array,
            },
            Property {
                position: Position::PlanEntry,
                key: "step",
                kind: ValueKind::String,
            },
            Property {
                position: Position::PlanEntry,
                key: "status",
                kind: ValueKind::Choice(&["pending", "in_progress", "completed"]),
            },
        ],
    },
    PublicToolSchema {
        version: "codex.exec_command.v1",
        aliases: &["exec_command", "functions.exec_command"],
        properties: &[
            Property {
                position: Position::Arguments,
                key: "cmd",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "workdir",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "yield_time_ms",
                kind: ValueKind::Unsigned,
            },
            Property {
                position: Position::Arguments,
                key: "max_output_tokens",
                kind: ValueKind::Unsigned,
            },
            Property {
                position: Position::Arguments,
                key: "login",
                kind: ValueKind::Boolean,
            },
            Property {
                position: Position::Arguments,
                key: "shell",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "tty",
                kind: ValueKind::Boolean,
            },
            Property {
                position: Position::Arguments,
                key: "sandbox_permissions",
                kind: ValueKind::Choice(&["use_default", "require_escalated"]),
            },
            Property {
                position: Position::Arguments,
                key: "justification",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "prefix_rule",
                kind: ValueKind::Array,
            },
        ],
    },
    PublicToolSchema {
        version: "codex.write_stdin.v1",
        aliases: &["write_stdin", "functions.write_stdin"],
        properties: &[
            Property {
                position: Position::Arguments,
                key: "session_id",
                kind: ValueKind::Unsigned,
            },
            Property {
                position: Position::Arguments,
                key: "chars",
                kind: ValueKind::String,
            },
            Property {
                position: Position::Arguments,
                key: "yield_time_ms",
                kind: ValueKind::Unsigned,
            },
            Property {
                position: Position::Arguments,
                key: "max_output_tokens",
                kind: ValueKind::Unsigned,
            },
        ],
    },
];

#[cfg(feature = "near-ai-privacy-filter")]
#[derive(Clone, Copy)]
pub(super) struct KeyContext {
    hash: [u8; 32],
    position: Position,
}
#[cfg(feature = "near-ai-privacy-filter")]
impl KeyContext {
    fn root() -> Self {
        Self {
            hash: Sha256::digest(b"trace_commons.review_key_location.v1").into(),
            position: Position::Other,
        }
    }
    fn child(self, tag: u8, bytes: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(b"trace_commons.review_key_location_child.v1");
        h.update(self.hash);
        h.update([tag]);
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
        Self {
            hash: h.finalize().into(),
            position: Position::Other,
        }
    }
    fn key(self, key: &str) -> Self {
        self.child(b'k', key.as_bytes())
    }
    fn index(self, index: usize) -> Self {
        self.child(b'i', &(index as u64).to_be_bytes())
    }
}

#[cfg(feature = "near-ai-privacy-filter")]
pub(super) struct ReviewKeyScanner {
    pub diagnostics: ReviewKeyDiagnostics,
    context: KeyContext,
    schema: Option<&'static PublicToolSchema>,
    sweeper: DeterministicTraceRedactor,
}
#[cfg(feature = "near-ai-privacy-filter")]
impl Default for ReviewKeyScanner {
    fn default() -> Self {
        Self {
            diagnostics: ReviewKeyDiagnostics::default(),
            context: KeyContext::root(),
            schema: None,
            sweeper: DeterministicTraceRedactor::bare(),
        }
    }
}
#[cfg(feature = "near-ai-privacy-filter")]
impl ReviewKeyScanner {
    pub fn begin_event(&mut self, index: usize, event: &TraceContributionEvent) {
        self.context = KeyContext::root()
            .key("events")
            .index(index)
            .key("structured_payload");
        self.context.position = Position::PayloadRoot;
        self.schema = (event.event_type == TraceContributionEventType::ToolCall)
            .then(|| {
                event.tool_name.as_deref().and_then(|name| {
                    PUBLIC_TOOL_SCHEMAS
                        .iter()
                        .find(|schema| schema.aliases.contains(&name))
                })
            })
            .flatten();
    }
    pub fn begin_side(&mut self, container: &str, field: &str) {
        self.context = KeyContext::root().key(container).key(field);
        self.schema = None;
    }
    pub fn push_key(&mut self, key: &str) -> KeyContext {
        let old = self.context;
        self.context = old.key(key);
        self.context.position = match (old.position, key) {
            (Position::PayloadRoot, "arguments") => Position::Arguments,
            (Position::Arguments, "plan") => Position::PlanArray,
            _ => Position::Other,
        };
        old
    }
    pub fn push_index(&mut self, index: usize) -> KeyContext {
        let old = self.context;
        self.context = old.index(index);
        if matches!(old.position, Position::PlanArray) {
            self.context.position = Position::PlanEntry;
        }
        old
    }
    pub fn restore(&mut self, context: KeyContext) {
        self.context = context;
    }
    fn recognized(&self, key: &str, value: Option<&Value>) -> bool {
        let (Some(schema), Some(value)) = (self.schema, value) else {
            return false;
        };
        schema.properties.iter().any(|property| {
            property.position == self.context.position
                && property.key == key
                && property.kind.matches(value)
        })
    }

    fn note(
        &mut self,
        key: &str,
        category: &str,
        confidence: Option<u32>,
        reason: ReviewKeyReason,
    ) {
        if self.diagnostics.findings.len() >= MAX_KEY_FINDINGS {
            self.diagnostics.truncated = true;
            return;
        }
        let category = safe_privacy_filter_label(Some(category), &mut RedactionReport::default());
        self.diagnostics.findings.push(ReviewKeyFinding {
            field_hash: format!("sha256:{}", hex::encode(self.context.key(key).hash)),
            category,
            confidence_millionths: confidence.filter(|v| *v <= 1_000_000),
            schema_version: self.schema.map(|schema| schema.version.to_string()),
            reason,
        });
    }
    pub fn note_deterministic(&mut self, key: &str, secret: bool) {
        self.note(
            key,
            if secret { "secret" } else { "unknown" },
            None,
            ReviewKeyReason::DeterministicFinding,
        );
    }
    pub fn note_scan_failure(&mut self, reason: ReviewKeyReason) {
        self.note("", "unknown", None, reason);
    }
    pub async fn inspect(
        &mut self,
        adapter: &dyn PrivacyFilterAdapter,
        key: &str,
        value: Option<&Value>,
        budget: &mut StructuredPayloadBudget,
        report: &mut RedactionReport,
    ) -> Result<bool, TraceContributionError> {
        // Even a catalog literal is never exempt from the local secret guard.
        let credentials = self.sweeper.detect_correction_credentials(key);
        let (_, local) = self.sweeper.redact_text(key);
        if credentials.blocked_secret_detected
            || credentials.key_finding_detected
            || local.counts.keys().any(|l| label_bears_severity(l))
            || local
                .pii_labels_present
                .iter()
                .any(|l| label_bears_severity(l))
        {
            report.key_finding_detected = true;
            self.note(
                key,
                if credentials.blocked_secret_detected || local.blocked_secret_detected {
                    "secret"
                } else {
                    "unknown"
                },
                None,
                ReviewKeyReason::DeterministicFinding,
            );
            return Ok(false);
        }
        if self.recognized(key, value) {
            if !budget.charge_field(key.len()) {
                report.coverage_incomplete = true;
                self.note(key, "unknown", None, ReviewKeyReason::BudgetExceeded);
                return Ok(false);
            }
            self.diagnostics.schema_keys_recognized =
                self.diagnostics.schema_keys_recognized.saturating_add(1);
            return Ok(true);
        }
        // Serde escapes controls/quotes; the key is data, never an instruction.
        // No values or neighbouring contributor text are sent with this key.
        let encoded =
            serde_json::to_string(key).map_err(|_| TraceContributionError::RedactionFailed {
                reason: "key context serialization failed".into(),
            })?;
        let context = format!("{{{encoded}:null}}");
        if !budget.charge_field(context.len()) {
            report.coverage_incomplete = true;
            self.note(key, "unknown", None, ReviewKeyReason::BudgetExceeded);
            return Ok(false);
        }
        self.diagnostics.contextual_keys_examined =
            self.diagnostics.contextual_keys_examined.saturating_add(1);
        let Some(answer) = adapter.redact_text(&context).await? else {
            report.coverage_incomplete = true;
            self.note(key, "unknown", None, ReviewKeyReason::ClassifierUnavailable);
            return Ok(false);
        };
        let nonempty = answer.summary.span_count > 0
            || !answer.report.counts.is_empty()
            || !answer.report.pii_labels_present.is_empty()
            || answer.report.blocked_secret_detected
            || answer.report.key_finding_detected
            || answer.redacted_text != context
            || answer.summary.by_label.values().any(|v| *v > 0);
        if answer.summary.decoded_mismatch || answer.report.coverage_incomplete {
            report.coverage_incomplete = true;
            self.note(key, "unknown", None, ReviewKeyReason::InvalidEvidence);
            return Ok(false);
        }
        let findings = answer
            .private_edits
            .as_ref()
            .map(|edits| edits.1.as_slice())
            .unwrap_or_default();
        let edits = answer
            .private_edits
            .as_ref()
            .map(|edits| edits.0.as_slice())
            .unwrap_or_default();
        if answer.private_edits.is_none() {
            report.key_finding_detected |= nonempty;
            report.coverage_incomplete = true;
            self.note(key, "unknown", None, ReviewKeyReason::InvalidEvidence);
            return Ok(false);
        }
        if !nonempty && findings.is_empty() && edits.is_empty() {
            return Ok(true);
        }
        report.key_finding_detected = true;
        let token_start = 1usize;
        let token_end = 1 + encoded.len();
        let valid_range = |start: u64, end: u64| -> bool {
            let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
                return false;
            };
            start < end
                && start >= token_start
                && end <= token_end
                && context.is_char_boundary(start)
                && context.is_char_boundary(end)
        };
        let mut rebuilt = Vec::new();
        let mut cursor = 0usize;
        let mut valid = !findings.is_empty()
            && findings.len() == answer.summary.span_count as usize
            && !edits.is_empty();
        for edit in edits {
            if !valid_range(edit.original.start, edit.original.end) {
                valid = false;
                break;
            }
            let start = edit.original.start as usize;
            let end = edit.original.end as usize;
            if start < cursor {
                valid = false;
                break;
            }
            rebuilt.extend_from_slice(&context.as_bytes()[cursor..start]);
            rebuilt.extend_from_slice(&edit.replacement);
            cursor = end;
        }
        if valid {
            rebuilt.extend_from_slice(&context.as_bytes()[cursor..]);
            valid = rebuilt == answer.redacted_text.as_bytes();
        }
        valid &= findings.iter().all(|f| {
            valid_range(f.original.start, f.original.end)
                && f.confidence_millionths.is_none_or(|v| v <= 1_000_000)
        });
        if !valid {
            report.coverage_incomplete = true;
            self.note(key, "unknown", None, ReviewKeyReason::InvalidEvidence);
            return Ok(false);
        }
        for finding in findings {
            self.note(
                key,
                &finding.category,
                finding.confidence_millionths,
                ReviewKeyReason::ClassifierFinding,
            );
        }
        Ok(false)
    }
}

#[cfg(all(test, feature = "near-ai-privacy-filter"))]
#[path = "key_privacy_tests.rs"]
mod tests;
