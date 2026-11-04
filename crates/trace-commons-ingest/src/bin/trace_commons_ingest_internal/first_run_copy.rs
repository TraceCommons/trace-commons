// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/public/first-run-copy`: the lines the first run shows under
//! the traces it found (`trace_commons_protocol::first_run_copy`).
//!
//! Unauthenticated and outside tenant context, like
//! `/v1/public/first-run-estimate`: the first run asks before an account or
//! any consent exists. The body carries two figures the client has already
//! coarsened; anything else, or a figure that is not coarse, is a 400 and
//! the client shows its own lines. Each request served or refused here is
//! stored as one row for operator statistics (V153,
//! `trace_commons_server::db::first_run_stats`): the hour, the coarsened
//! figures, the answer and its lines' key, and the app version and platform
//! when sent; never an address or any identifier. A failed write is logged
//! by label and the answer goes out anyway. The log line carries two bucket
//! labels and which lines were served, never a figure.
//!
//! `GET /v1/admin/first-run-estimate-stats?days=N` (admin) reads the
//! aggregates back.
//!
//! By default the answer is the protocol's default lines, the same lines a
//! client shows when it cannot reach the commons. An operator can instead
//! serve a dollar line, worded by the operator, from four settings that are
//! all set or all unset ([`FIRST_RUN_COPY_USD_ENVS`]). The amount is the
//! traces sent times the per-trace figure, capped at the account ceiling.
//! These settings are separate from the first-run estimate's
//! `TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_*`, which configure a
//! different route.

use axum::body::Bytes;
use sha2::{Digest as _, Sha256};
use trace_commons_protocol::first_run_copy::{
    DEFAULT_LINES_KEY, FirstRunCopyLines, FirstRunCopyRequest, MIB, default_lines, valid_line,
};
use trace_commons_protocol::first_run_estimate::{FIRST_RUN_USD_MICROS_MAX, USD_MICROS_PER_CENT};
use trace_commons_server::db::first_run_stats::{
    FirstRunEstimateOutcome, FirstRunEstimateRecord, FirstRunEstimateStats,
};

use super::*;

/// Per-trace dollar figure, in micro-dollars, for the dollar line.
pub(super) const FIRST_RUN_COPY_USD_PER_TRACE_ENV: &str =
    "TRACE_COMMONS_FIRST_RUN_COPY_USD_MICROS_PER_TRACE";
/// The most one account can be paid, in micro-dollars: the amount's cap.
pub(super) const FIRST_RUN_COPY_USD_CEILING_ENV: &str =
    "TRACE_COMMONS_FIRST_RUN_COPY_USD_ACCOUNT_CEILING_MICROS";
/// The headline while the use that earns credit is on. Holds `{amount}`
/// once.
pub(super) const FIRST_RUN_COPY_USD_LINE_ENV: &str = "TRACE_COMMONS_FIRST_RUN_COPY_USD_LINE";
/// The headline while that use is off. Holds `{amount}` once.
pub(super) const FIRST_RUN_COPY_USD_COULD_LINE_ENV: &str =
    "TRACE_COMMONS_FIRST_RUN_COPY_USD_COULD_LINE";

/// The four dollar-line settings, all set or all unset.
pub(super) const FIRST_RUN_COPY_USD_ENVS: [&str; 4] = [
    FIRST_RUN_COPY_USD_PER_TRACE_ENV,
    FIRST_RUN_COPY_USD_CEILING_ENV,
    FIRST_RUN_COPY_USD_LINE_ENV,
    FIRST_RUN_COPY_USD_COULD_LINE_ENV,
];

/// Boot refusal for a dollar-line setting that is half set, not plain
/// digits, out of range, or a template that is not one valid line with one
/// `{amount}`. Never carries the value.
const FIRST_RUN_COPY_USD_INVALID: &str = "first_run_copy_usd_invalid";

/// The placeholder a dollar-line template holds once.
const AMOUNT: &str = "{amount}";

/// Per-IP cap on the route per window, as for `/v1/public/register-stats`.
const FIRST_RUN_COPY_PER_IP_LIMIT: u32 = 30;
/// The bound no header can escape.
const FIRST_RUN_COPY_GLOBAL_LIMIT: u32 = 1200;

/// What the route serves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct FirstRunCopyConfig {
    /// `None`: the default lines.
    pub(super) usd: Option<UsdLine>,
}

/// The operator's dollar line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UsdLine {
    per_trace_usd_micros: u64,
    ceiling_usd_micros: u64,
    line: String,
    could_line: String,
    /// `usd.` and the first 16 hex digits of the SHA-256 of the two
    /// templates: names the wording without storing it.
    key: String,
}

pub(super) fn config_from_env() -> anyhow::Result<Arc<FirstRunCopyConfig>> {
    let [per_trace, ceiling, line, could_line] =
        FIRST_RUN_COPY_USD_ENVS.map(|name| std::env::var(name).ok());
    config_from_values(
        per_trace.as_deref(),
        ceiling.as_deref(),
        line.as_deref(),
        could_line.as_deref(),
    )
    .map(Arc::new)
}

/// All four unset is the default lines. All four set must be a per-trace
/// figure and a ceiling in whole cents within the protocol's bound, and two
/// templates that each hold `{amount}` once and are a valid line with an
/// amount in it. Anything else refuses boot.
pub(super) fn config_from_values(
    per_trace: Option<&str>,
    ceiling: Option<&str>,
    line: Option<&str>,
    could_line: Option<&str>,
) -> anyhow::Result<FirstRunCopyConfig> {
    let config = match (per_trace, ceiling, line, could_line) {
        (None, None, None, None) => FirstRunCopyConfig::default(),
        (Some(per_trace), Some(ceiling), Some(line), Some(could_line)) => {
            let micros = |raw: &str| -> anyhow::Result<u64> {
                if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                    anyhow::bail!(FIRST_RUN_COPY_USD_INVALID);
                }
                let value: u64 = raw
                    .parse()
                    .map_err(|_| anyhow::anyhow!(FIRST_RUN_COPY_USD_INVALID))?;
                if value == 0 || !value.is_multiple_of(USD_MICROS_PER_CENT) {
                    anyhow::bail!(FIRST_RUN_COPY_USD_INVALID);
                }
                Ok(value)
            };
            let per_trace = micros(per_trace)?;
            let ceiling = micros(ceiling)?;
            if per_trace > FIRST_RUN_USD_MICROS_MAX || per_trace > ceiling {
                anyhow::bail!(FIRST_RUN_COPY_USD_INVALID);
            }
            for template in [line, could_line] {
                if template.matches(AMOUNT).count() != 1
                    || !valid_line(&template.replace(AMOUNT, &usd_figure(ceiling)))
                {
                    anyhow::bail!(FIRST_RUN_COPY_USD_INVALID);
                }
            }
            let digest = Sha256::new()
                .chain_update(line.as_bytes())
                .chain_update([0])
                .chain_update(could_line.as_bytes())
                .finalize();
            FirstRunCopyConfig {
                usd: Some(UsdLine {
                    per_trace_usd_micros: per_trace,
                    ceiling_usd_micros: ceiling,
                    line: line.to_owned(),
                    could_line: could_line.to_owned(),
                    key: format!("usd.{}", &hex::encode(digest)[..16]),
                }),
            }
        }
        _ => anyhow::bail!(FIRST_RUN_COPY_USD_INVALID),
    };
    tracing::info!(
        lines = if config.usd.is_some() {
            "usd"
        } else {
            "default"
        },
        "first-run copy configured"
    );
    Ok(config)
}

/// What was served: the label of which lines (`default` or `usd`) and the
/// lines' key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Served {
    pub(super) variant: &'static str,
    pub(super) lines_key: String,
}

impl Served {
    fn default_lines() -> Self {
        Self {
            variant: "default",
            lines_key: DEFAULT_LINES_KEY.to_owned(),
        }
    }
}

/// The lines for a valid request, and which they are.
pub(super) fn lines_for(
    config: &FirstRunCopyConfig,
    request: &FirstRunCopyRequest,
) -> (FirstRunCopyLines, Served) {
    let defaults = default_lines();
    let Some(usd) = config.usd.as_ref() else {
        return (defaults, Served::default_lines());
    };
    let amount = request
        .trace_count
        .saturating_mul(usd.per_trace_usd_micros)
        .min(usd.ceiling_usd_micros);
    if amount == 0 {
        return (defaults, Served::default_lines());
    }
    let figure = usd_figure(amount);
    let lines = FirstRunCopyLines {
        found_credit: usd.line.replace(AMOUNT, &figure),
        found_could_credit: usd.could_line.replace(AMOUNT, &figure),
        credit_depends: defaults.credit_depends,
    };
    // Validated at boot with the ceiling, the longest figure; checked again
    // so a figure can never make the route serve a line a client refuses.
    if lines.is_valid() {
        (
            lines,
            Served {
                variant: "usd",
                lines_key: usd.key.clone(),
            },
        )
    } else {
        (default_lines(), Served::default_lines())
    }
}

/// One dollar figure from micro-dollars, cut to cents: "$0.40", "$1",
/// "$1,030.75".
pub(super) fn usd_figure(usd_micros: u64) -> String {
    let dollars = usd_micros / 1_000_000;
    let cents = (usd_micros % 1_000_000) / USD_MICROS_PER_CENT;
    let digits = dollars.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if cents == 0 {
        format!("${grouped}")
    } else {
        format!("${grouped}.{cents:02}")
    }
}

/// The two labels the log line carries: an order-of-magnitude bucket for
/// each figure, from a fixed set. Never a figure.
pub(super) fn log_buckets(request: &FirstRunCopyRequest) -> (&'static str, &'static str) {
    const TRACES: [&str; 7] = [
        "traces_0",
        "traces_1_9",
        "traces_10_99",
        "traces_100_999",
        "traces_1k_9k",
        "traces_10k_99k",
        "traces_100k_plus",
    ];
    const SIZE: [&str; 7] = [
        "mib_0",
        "mib_1_9",
        "mib_10_99",
        "mib_100_999",
        "mib_1k_9k",
        "mib_10k_99k",
        "mib_100k_plus",
    ];
    let bucket = |value: u64| -> usize {
        match value {
            0 => 0,
            1..=9 => 1,
            10..=99 => 2,
            100..=999 => 3,
            1_000..=9_999 => 4,
            10_000..=99_999 => 5,
            _ => 6,
        }
    };
    (
        TRACES[bucket(request.trace_count)],
        SIZE[bucket(request.total_bytes / MIB)],
    )
}

/// The request, or `None` for anything a client following the protocol
/// would not send: not the JSON object, an unknown or missing field, or a
/// figure that is not coarse or out of bounds.
pub(super) fn parse_request(body: &[u8]) -> Option<FirstRunCopyRequest> {
    let request: FirstRunCopyRequest = serde_json::from_slice(body).ok()?;
    request.is_valid().then_some(request)
}

/// Answers `POST /v1/public/first-run-copy`. See the module docs.
pub(super) async fn first_run_copy_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let client_ip = client_ip_for_rate_limit(&headers);
    if !ACCOUNT_RATE_LIMITER.check(
        &format!("first-run-copy-ip:{client_ip}"),
        FIRST_RUN_COPY_PER_IP_LIMIT,
    ) || !ACCOUNT_RATE_LIMITER.check_global("first-run-copy-global", FIRST_RUN_COPY_GLOBAL_LIMIT)
    {
        tracing::info!(outcome = "rate_limited", "first-run copy");
        return api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let Some(request) = parse_request(&body) else {
        tracing::info!(outcome = "bad_request", "first-run copy");
        record(
            &state,
            FirstRunEstimateRecord {
                outcome: FirstRunEstimateOutcome::Refused {
                    label: REQUEST_INVALID,
                },
                app_version: None,
                platform: None,
            },
        )
        .await;
        return api_error(StatusCode::BAD_REQUEST, REQUEST_INVALID).into_response();
    };
    let (traces, size) = log_buckets(&request);
    let (lines, served) = lines_for(&state.first_run_copy, &request);
    tracing::info!(
        outcome = "served",
        traces,
        size,
        lines = served.variant,
        "first-run copy"
    );
    record(&state, served_record(&request, served)).await;
    let mut response = Json(lines).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// The refusal label of a request a client following the protocol would
/// not send.
const REQUEST_INVALID: &str = "first_run_copy_request_invalid";

/// The row for a served request: the figures as received, the answer and
/// its lines' key, and the client's version and platform when sent.
pub(super) fn served_record(
    request: &FirstRunCopyRequest,
    served: Served,
) -> FirstRunEstimateRecord {
    FirstRunEstimateRecord {
        outcome: FirstRunEstimateOutcome::Served {
            variant: served.variant,
            lines_key: served.lines_key,
            trace_count: request.trace_count,
            total_bytes: request.total_bytes,
        },
        app_version: request.app_version.clone(),
        platform: request.platform.clone(),
    }
}

/// Store one row, when a store is configured. A failure is logged by label
/// and never stops the answer.
async fn record(state: &AppState, record: FirstRunEstimateRecord) {
    let Some(store) = state.first_run_stats.as_ref() else {
        return;
    };
    if store.record_first_run_estimate(&record).await.is_err() {
        tracing::warn!(
            label = "first_run_copy_record_failed",
            "first-run copy request not recorded"
        );
    }
}

/// The most days the statistics route reads back, and its default.
const STATS_MAX_DAYS: i64 = 366;
const STATS_DEFAULT_DAYS: i64 = 30;

/// `GET /v1/admin/first-run-estimate-stats` query.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatsQuery {
    /// Days back from now, 1 to [`STATS_MAX_DAYS`]; [`STATS_DEFAULT_DAYS`]
    /// when absent.
    #[serde(default)]
    days: Option<i64>,
}

/// Answers `GET /v1/admin/first-run-estimate-stats`: the aggregates over the
/// first run's estimate requests since `days` ago. Admin only.
pub(super) async fn stats_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StatsQuery>,
) -> ApiResult<Json<FirstRunEstimateStats>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let days = query.days.unwrap_or(STATS_DEFAULT_DAYS);
    if !(1..=STATS_MAX_DAYS).contains(&days) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "first_run_estimate_stats_days_out_of_range",
        ));
    }
    let Some(store) = state.first_run_stats.as_ref() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "first-run estimate stats require a configured database",
        ));
    };
    let since = Utc::now() - chrono::Duration::days(days);
    let stats = store.first_run_estimate_stats(since).await.map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "first_run_estimate_stats_unavailable",
        )
    })?;
    Ok(Json(stats))
}
