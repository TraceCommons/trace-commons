use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Top-level trace file - extended format with memory snapshot and HTTP exchanges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceFile {
    pub model_name: String,
    /// Workspace memory documents captured before the recording session.
    /// Replay should restore these before running the trace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_snapshot: Vec<MemorySnapshotEntry>,
    /// HTTP exchanges recorded during the session, in order.
    /// Replay should return these instead of making real HTTP requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_exchanges: Vec<HttpExchange>,
    pub steps: Vec<TraceStep>,
}

/// A memory document captured at recording start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshotEntry {
    pub path: String,
    pub content: String,
}

/// A recorded HTTP request/response pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchange {
    pub request: HttpExchangeRequest,
    pub response: HttpExchangeResponse,
}

/// The request side of an HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchangeRequest {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// The response side of an HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchangeResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// A single step in the trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_hint: Option<RequestHint>,
    pub response: TraceResponse,
    /// Tool results that appeared in the message context since the previous step.
    /// During replay, the test harness can compare actual tool results against
    /// these to verify tool output hasn't changed (regression detection).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_tool_results: Vec<ExpectedToolResult>,
    /// When this step happened, where the recording knows.
    ///
    /// `None` on every recording written before this field existed, and on
    /// sources that do not carry per-step times. `from_recorded_trace` falls
    /// back to the envelope's `created_at` in that case rather than
    /// synthesising an offset: a plausible-looking invented timestamp is
    /// worse than an absent one, because a consumer cannot tell it apart
    /// from a real one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

/// Soft validation hints for matching a step to a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_message_contains: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_message_count: Option<usize>,
}

/// Tagged response enum — text, tool_calls, or user_input.
///
/// `user_input` steps are metadata markers — they record what the user said
/// but do **not** correspond to an LLM call. During replay, `TraceLlm` must
/// skip `user_input` steps and only consume `text`/`tool_calls` steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceResponse {
    Text {
        content: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    ToolCalls {
        tool_calls: Vec<TraceToolCall>,
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Marker for a user message that triggered subsequent LLM calls.
    /// Not an LLM response — replay providers must skip these.
    UserInput { content: String },
}

/// A tool call in a trace step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Recorded tool result for regression checking during replay.
///
/// During replay, after tools execute and before returning the canned LLM
/// response, the test harness should compare actual `Role::Tool` messages
/// against these entries. A content mismatch indicates a tool behavior change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedToolResult {
    pub tool_call_id: String,
    pub name: String,
    /// The full tool result content as it appeared in the message context.
    pub content: String,
}
