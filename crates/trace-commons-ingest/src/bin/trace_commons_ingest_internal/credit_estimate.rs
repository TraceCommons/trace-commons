// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The local credit estimate table: where clients fetch it, and the admin
//! route that measures and fits it.
//!
//! `GET /v1/credit-estimate/table` is unauthenticated and outside tenant
//! context, like `/v1/source`: the table is public, non-personal data, and a
//! client fetches it before it has an account. It serves the table the
//! operator installed, validated at boot with the same parser the client
//! uses, or the protocol's built-in table. It never serves an empty table.
//!
//! `GET /v1/credit-estimate/settlement-table` is unauthenticated and outside
//! tenant context for the same reason. It serves the v2 settlement table the
//! operator installed through `TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH`,
//! validated at boot with the client's parser. There is no built-in
//! settlement table: with none installed the route answers 404 and clients
//! draw nothing.
//!
//! `GET /v1/public/first-run-estimate` is unauthenticated and outside
//! tenant context for the same reason. It serves the first run's dollar
//! estimate per accepted trace, from two operator settings
//! ([`FIRST_RUN_USD_MIN_ENV`], [`FIRST_RUN_USD_MAX_ENV`]) parsed fail-closed
//! at boot, or `{}` when neither is set.
//!
//! `POST /v1/admin/credit-estimate-eval` returns one label-only row per
//! labelled decision: the `lef1` features of the stored envelope, the
//! displayed credit (2 decimals), its calibration version, the withheld
//! label, and a tenant tag. The features are derived inside the gate
//! service, so this handler never holds plaintext. Rows carry no submission
//! id, trace id, decision time or content, and are shuffled. They also carry
//! nothing stored beside a submission id that would join them back to it:
//! not the exact credit quality, and not a tenant hash, since an unsalted
//! hash of a tenant id is recomputable by anyone holding the id. The tenant
//! tag is assigned per run in a random order and only says which rows share
//! a tenant. With `fit=true` it also runs the E9 time split
//! and returns the metrics and a candidate table. It writes nothing.
//!
//! With `target=settlement` the same rows are labelled instead with the
//! settled `trace_credit` award of the pipeline run that wrote the
//! submission's gate decision: its `atomic_units`, 0 for a run settled with
//! no award, and left out (counted unlabelled) when the run is held, not
//! complete, or has no Score outcome. With `fit=true` it fits a settlement
//! table (`fit_settlement_table`), drawing its batches from `seed`
//! (default 0) so a run is reproducible. The candidate carries no dollar
//! rate and no pricing version; the operator adds both.

use std::path::Path;

use rand::seq::SliceRandom as _;
use trace_commons_protocol::first_run_estimate::{FirstRunEstimate, UsdMicrosRange};
use trace_commons_protocol::local_credit_estimate::LocalEstimateTable;
use trace_commons_protocol::settlement_estimate::SettlementEstimateTable;
use trace_commons_server::credit_estimate_fit::{
    EstimateEvalRow, EstimateFitInput, EstimateFitReport, SettlementEvalRow, SettlementFitInput,
    SettlementFitReport, displayed_credit, eval_label, fit_estimate_table, fit_settlement_table,
};
use trace_commons_server::trace_corpus_storage::SettlementLabel;

use super::*;

/// Path of an operator-installed table document. Unset serves the built-in
/// table.
pub(super) const CREDIT_ESTIMATE_TABLE_PATH_ENV: &str = "TRACE_COMMONS_CREDIT_ESTIMATE_TABLE_PATH";

/// Refusal label for an installed table that cannot be read or that the
/// client's parser would refuse. Boot fails with it, as it does for an
/// invalid activity-missions policy, so a bad file is caught at deploy
/// rather than served.
const TABLE_INVALID: &str = "credit_estimate_table_invalid";

/// The most decisions one eval run enumerates, and the default when no
/// `limit` is given. Each labelled submission is decrypted and derived
/// within the one request and every row is held in memory, so a run is
/// bounded rather than sized by the corpus. A larger `limit` is refused.
pub(super) const CREDIT_ESTIMATE_EVAL_MAX_LIMIT: i64 = 10_000;

pub(super) fn table_from_env() -> anyhow::Result<Arc<LocalEstimateTable>> {
    let path = std::env::var_os(CREDIT_ESTIMATE_TABLE_PATH_ENV).map(std::path::PathBuf::from);
    table_from_path(path.as_deref()).map(Arc::new)
}

pub(super) fn table_from_path(path: Option<&Path>) -> anyhow::Result<LocalEstimateTable> {
    let Some(path) = path else {
        return Ok(LocalEstimateTable::built_in());
    };
    let raw = std::fs::read(path).map_err(|_| anyhow::anyhow!(TABLE_INVALID))?;
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| anyhow::anyhow!(TABLE_INVALID))?;
    LocalEstimateTable::from_value(&value).map_err(|_| anyhow::anyhow!(TABLE_INVALID))
}

/// Answers `GET /v1/credit-estimate/table`. See the module docs.
pub(super) async fn table_handler(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let mut response = Json(state.credit_estimate_table.as_ref().clone()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Path of an operator-installed settlement table document. Unset serves
/// no settlement table at all.
pub(super) const CREDIT_SETTLEMENT_TABLE_PATH_ENV: &str =
    "TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH";

/// Refusal label for an installed settlement table that cannot be read,
/// exceeds the size limit, or that the client's parser would refuse. Boot
/// fails with it.
const SETTLEMENT_TABLE_INVALID: &str = "credit_settlement_table_invalid";

pub(super) fn settlement_table_from_env() -> anyhow::Result<Option<Arc<SettlementEstimateTable>>> {
    let path = std::env::var_os(CREDIT_SETTLEMENT_TABLE_PATH_ENV).map(std::path::PathBuf::from);
    Ok(settlement_table_from_path(path.as_deref())?.map(Arc::new))
}

/// `None` path is no table. Otherwise the file is read and parsed with
/// [`SettlementEstimateTable::from_slice`], which refuses a document past
/// the size limit before parsing it; any failure is the one label.
pub(super) fn settlement_table_from_path(
    path: Option<&Path>,
) -> anyhow::Result<Option<SettlementEstimateTable>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let raw = std::fs::read(path).map_err(|_| anyhow::anyhow!(SETTLEMENT_TABLE_INVALID))?;
    SettlementEstimateTable::from_slice(&raw)
        .map(Some)
        .map_err(|_| anyhow::anyhow!(SETTLEMENT_TABLE_INVALID))
}

/// Answers `GET /v1/credit-estimate/settlement-table`. See the module docs.
pub(super) async fn settlement_table_handler(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let Some(table) = state.credit_settlement_table.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Json(table.as_ref().clone()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// The first run's dollar estimate per accepted trace, in micro-dollars
/// (`GET /v1/public/first-run-estimate`). Set both or neither. Unset, the
/// route serves `{}` and clients word the first run in points.
pub(super) const FIRST_RUN_USD_MIN_ENV: &str = "TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MIN";
pub(super) const FIRST_RUN_USD_MAX_ENV: &str = "TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MAX";

/// Refusal label for a first-run estimate setting that is half set, not
/// plain digits, or outside [`UsdMicrosRange::is_valid`]. Boot fails with it;
/// it never carries the value.
const FIRST_RUN_USD_INVALID: &str = "first_run_usd_estimate_invalid";

pub(super) fn first_run_estimate_from_env() -> anyhow::Result<Arc<FirstRunEstimate>> {
    let min = std::env::var(FIRST_RUN_USD_MIN_ENV).ok();
    let max = std::env::var(FIRST_RUN_USD_MAX_ENV).ok();
    first_run_estimate_from_values(min.as_deref(), max.as_deref()).map(Arc::new)
}

/// Both unset is no range. Both set must each be plain ASCII digits and
/// together a range [`UsdMicrosRange::is_valid`] accepts. Anything else,
/// including one set without the other or an empty value, refuses.
pub(super) fn first_run_estimate_from_values(
    min: Option<&str>,
    max: Option<&str>,
) -> anyhow::Result<FirstRunEstimate> {
    let parse = |raw: &str| -> anyhow::Result<u64> {
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            anyhow::bail!(FIRST_RUN_USD_INVALID);
        }
        raw.parse::<u64>()
            .map_err(|_| anyhow::anyhow!(FIRST_RUN_USD_INVALID))
    };
    match (min, max) {
        (None, None) => Ok(FirstRunEstimate::default()),
        (Some(min), Some(max)) => {
            let range = UsdMicrosRange {
                min: parse(min)?,
                max: parse(max)?,
            };
            if !range.is_valid() {
                anyhow::bail!(FIRST_RUN_USD_INVALID);
            }
            Ok(FirstRunEstimate {
                usd_micros_per_accepted_trace: Some(range),
            })
        }
        _ => anyhow::bail!(FIRST_RUN_USD_INVALID),
    }
}

/// Answers `GET /v1/public/first-run-estimate`: unauthenticated, outside
/// tenant context, public operator configuration. A short cache, so an
/// operator's change reaches new first runs within minutes.
pub(super) async fn first_run_estimate_handler(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let mut response = Json(state.first_run_estimate.as_ref().clone()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Query for the eval route. A mistyped name is refused rather than read as
/// its default.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreditEstimateEvalQuery {
    /// Enumerate and read labels only: no envelope is loaded or decrypted,
    /// and no row is returned. For seeing the size of a run before paying
    /// for it.
    #[serde(default)]
    pub(super) dry_run: bool,
    /// Bounds the decisions enumerated, newest first. Defaults to, and
    /// may not exceed, [`CREDIT_ESTIMATE_EVAL_MAX_LIMIT`].
    #[serde(default)]
    pub(super) limit: Option<i64>,
    /// Also fit and evaluate a candidate table.
    #[serde(default)]
    pub(super) fit: bool,
    /// Which label the rows carry and which table a fit produces.
    #[serde(default)]
    pub(super) target: EstimateEvalTarget,
    /// Seeds the settlement fit's batch draw. Defaults to 0.
    #[serde(default)]
    pub(super) seed: Option<u64>,
}

/// What an eval run labels rows with.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EstimateEvalTarget {
    /// Displayed credit, for the v1 display table. Today's behaviour.
    #[default]
    Display,
    /// The settled `trace_credit` award, for the v2 settlement table.
    Settlement,
}

impl EstimateEvalTarget {
    fn label(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Settlement => "settlement",
        }
    }
}

/// Counts for one eval run. Aggregates only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct CreditEstimateEvalCounts {
    /// Decision rows enumerated.
    pub(super) decisions: usize,
    /// Distinct submissions among them; each is labelled by its latest
    /// decision.
    pub(super) submissions: usize,
    /// Submissions with a credit quality or a withheld reason; under
    /// `target=settlement`, with a settled award or a settlement at zero.
    pub(super) labelled: usize,
    /// Submissions with neither: still being scored, or under
    /// `target=settlement` excluded (held, unfinished, unscored). Left out.
    pub(super) unlabelled: usize,
    /// Labelled submissions whose features were derived.
    pub(super) derived: usize,
    /// Labelled submissions whose envelope could not be loaded or whose
    /// features the gate service could not derive. Left out, never zeroed.
    pub(super) failed: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct CreditEstimateEvalResponse {
    pub(super) dry_run: bool,
    pub(super) counts: CreditEstimateEvalCounts,
    /// Shuffled, label-only. Empty under `target=settlement`.
    pub(super) rows: Vec<EstimateEvalRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) fit: Option<EstimateFitReport>,
    /// Shuffled, label-only; only under `target=settlement`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) settlement_rows: Vec<SettlementEvalRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) settlement_fit: Option<SettlementFitReport>,
    /// Failed submissions counted by reason ([`derive_failure_reason`]):
    /// a content-free parse reason, or an error hash. Never a submission.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) failed_reasons: BTreeMap<String, usize>,
}

/// Why one submission's features were not derived, safe to log and return.
/// An envelope parse failure carries its schema-only reason; anything else
/// (a load or decrypt error, whose text can name storage) is hashed.
pub(super) fn derive_failure_reason(error: &anyhow::Error) -> String {
    let text = error.to_string();
    if text.starts_with(
        trace_commons_server::trace_gate_service::ESTIMATE_FEATURES_ENVELOPE_UNREADABLE,
    ) {
        text
    } else {
        safe_display_error_hash(error)
    }
}

/// One submission's label, whichever target the run reads.
enum RowLabel {
    Display {
        credit_quality_micros: Option<i64>,
        calibration_version: Option<i32>,
        withheld: Option<trace_commons_server::credit_estimate_fit::EstimateWithheldLabel>,
    },
    Settlement(u64),
}

/// One eval run. Enumerates decisions cross-tenant on the gate-driver pool,
/// newest first (identifiers and decision times only), reads each tenant's labels through
/// the tenant-scoped pool (the calibration version and withheld reason are
/// not granted to the gate-driver role), and derives features for each
/// labelled submission inside the gate service from the envelope the corpus
/// holds now (rescrubbed first, else submitted).
pub(super) async fn run_credit_estimate_eval(
    state: &AppState,
    query: &CreditEstimateEvalQuery,
) -> anyhow::Result<CreditEstimateEvalResponse> {
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("credit estimate eval requires a configured DB mirror"))?;
    let limit = query
        .limit
        .unwrap_or(CREDIT_ESTIMATE_EVAL_MAX_LIMIT)
        .clamp(0, CREDIT_ESTIMATE_EVAL_MAX_LIMIT);
    let decisions = db.list_recent_gate_decision_keys(limit).await?;
    let mut counts = CreditEstimateEvalCounts {
        decisions: decisions.len(),
        ..CreditEstimateEvalCounts::default()
    };
    let mut failed_reasons: BTreeMap<String, usize> = BTreeMap::new();

    // Latest decision time per (tenant, submission), matching the label
    // read, which is latest-per-submission. The enumeration is newest
    // first, so every decision newer than the cut is in it: a submission it
    // reaches has its latest decision here, and the time split uses the
    // decision the label came from.
    let mut latest: BTreeMap<(String, Uuid), chrono::DateTime<Utc>> = BTreeMap::new();
    for row in &decisions {
        let slot = latest
            .entry((row.tenant_id.clone(), row.submission_id))
            .or_insert(row.decided_at);
        if row.decided_at > *slot {
            *slot = row.decided_at;
        }
    }
    counts.submissions = latest.len();
    let mut by_tenant: BTreeMap<&str, Vec<Uuid>> = BTreeMap::new();
    for (tenant_id, submission_id) in latest.keys() {
        by_tenant
            .entry(tenant_id.as_str())
            .or_default()
            .push(*submission_id);
    }

    // Per-run tenant tags, handed out in a random order so a tag carries
    // neither the tenant id nor its place in the enumeration.
    let tenant_tags: BTreeMap<&str, String> = {
        let mut tenants: Vec<&str> = by_tenant.keys().copied().collect();
        tenants.shuffle(&mut rand::thread_rng());
        tenants
            .into_iter()
            .enumerate()
            .map(|(i, tenant_id)| (tenant_id, format!("t{i}")))
            .collect()
    };

    let mut inputs: Vec<EstimateFitInput> = Vec::new();
    let mut settlement_inputs: Vec<SettlementFitInput> = Vec::new();
    for (tenant_id, submission_ids) in by_tenant {
        // Both reads are tenant-scoped; a submission with no label is
        // absent from the map and counted unlabelled.
        let labels: BTreeMap<Uuid, RowLabel> = match query.target {
            EstimateEvalTarget::Display => db
                .list_latest_gate_credit_decisions(tenant_id, &submission_ids)
                .await?
                .into_iter()
                .filter_map(|row| {
                    eval_label(
                        row.credit_quality_micros,
                        row.credit_withheld_reason.as_deref(),
                    )
                    .map(|withheld| {
                        (
                            row.submission_id,
                            RowLabel::Display {
                                credit_quality_micros: row.credit_quality_micros,
                                calibration_version: row.credit_quality_calibration_version,
                                withheld,
                            },
                        )
                    })
                })
                .collect(),
            // A held, unfinished or unscored run is excluded: no label.
            EstimateEvalTarget::Settlement => db
                .list_settlement_labels(tenant_id, &submission_ids)
                .await?
                .into_iter()
                .filter_map(|row| match row.label {
                    SettlementLabel::Settled(units) => {
                        Some((row.submission_id, RowLabel::Settlement(units)))
                    }
                    SettlementLabel::WithheldAtZero => {
                        Some((row.submission_id, RowLabel::Settlement(0)))
                    }
                    SettlementLabel::Excluded => None,
                })
                .collect(),
        };
        let tenant_hash = sha256_prefixed(tenant_id);
        for submission_id in submission_ids {
            let Some(label) = labels.get(&submission_id) else {
                counts.unlabelled += 1;
                continue;
            };
            counts.labelled += 1;
            if query.dry_run {
                continue;
            }
            let derived = async {
                let (ciphertext, wrapped_dek) =
                    load_trace_ciphertext_and_wrapped_dek(state, tenant_id, submission_id).await?;
                // Canonical tenant_storage_ref, as every gate path uses.
                let tenant_ctx = GateTenantCtx::from_canonical(tenant_storage_ref(tenant_id));
                // Decrypting and deriving is CPU work: off the async
                // runtime, so one eval run does not stall request handling.
                let gate_service = state.gate_service.clone();
                tokio::task::spawn_blocking(move || {
                    gate_service.derive_estimate_features(
                        &tenant_ctx,
                        &ciphertext,
                        &wrapped_dek,
                        TraceArtifactKind::ContributionEnvelope,
                    )
                })
                .await
                .map_err(|_| anyhow::anyhow!("credit estimate derive task failed"))?
            }
            .await;
            let features = match derived {
                Ok(features) => features,
                Err(error) => {
                    counts.failed += 1;
                    let reason = derive_failure_reason(&error);
                    tracing::warn!(
                        tenant_hash = %tenant_hash,
                        submission_hash = %sha256_prefixed(&submission_id.to_string()),
                        error_hash = %safe_display_error_hash(&error),
                        reason = %reason,
                        "credit estimate eval could not derive one submission"
                    );
                    *failed_reasons.entry(reason).or_default() += 1;
                    continue;
                }
            };
            counts.derived += 1;
            let tenant_tag = tenant_tags[tenant_id].clone();
            let decided_at = latest[&(tenant_id.to_string(), submission_id)];
            match label {
                RowLabel::Display {
                    credit_quality_micros,
                    calibration_version,
                    withheld,
                } => inputs.push(EstimateFitInput {
                    row: EstimateEvalRow {
                        features,
                        displayed_credit: credit_quality_micros.map(displayed_credit),
                        credit_quality_calibration_version: *calibration_version,
                        withheld: *withheld,
                        tenant_tag,
                    },
                    decided_at,
                }),
                RowLabel::Settlement(units) => settlement_inputs.push(SettlementFitInput {
                    row: SettlementEvalRow {
                        features,
                        settled_atomic_units: *units,
                        tenant_tag,
                    },
                    decided_at,
                }),
            }
        }
    }

    let fit = (query.fit && query.target == EstimateEvalTarget::Display)
        .then(|| fit_estimate_table(&inputs));
    let settlement_fit = (query.fit && query.target == EstimateEvalTarget::Settlement)
        .then(|| fit_settlement_table(&settlement_inputs, query.seed.unwrap_or(0), Utc::now()));
    // Enumeration order is decision-time order; shuffling keeps a row from
    // being matched back to a submission by its position. The row's values
    // are unlinkable on their own: see the module docs.
    let mut rows: Vec<EstimateEvalRow> = inputs.into_iter().map(|input| input.row).collect();
    rows.shuffle(&mut rand::thread_rng());
    let mut settlement_rows: Vec<SettlementEvalRow> = settlement_inputs
        .into_iter()
        .map(|input| input.row)
        .collect();
    settlement_rows.shuffle(&mut rand::thread_rng());
    Ok(CreditEstimateEvalResponse {
        dry_run: query.dry_run,
        counts,
        rows,
        fit,
        settlement_rows,
        settlement_fit,
        failed_reasons,
    })
}

/// Admin route: `POST /v1/admin/credit-estimate-eval`. See the module docs.
///
/// Auth: the admin bearer credential (`require_admin`), as the other
/// `/v1/admin/*` maintenance routes. Synchronous, because it returns rows;
/// `limit` defaults to and is capped at [`CREDIT_ESTIMATE_EVAL_MAX_LIMIT`].
pub(super) async fn eval_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CreditEstimateEvalQuery>,
) -> ApiResult<Json<CreditEstimateEvalResponse>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    if query.dry_run && query.fit {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "credit_estimate_eval_fit_needs_rows",
        ));
    }
    if query
        .limit
        .is_some_and(|limit| !(0..=CREDIT_ESTIMATE_EVAL_MAX_LIMIT).contains(&limit))
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "credit_estimate_eval_limit_out_of_range",
        ));
    }
    if state.db_mirror.is_none() {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "credit estimate eval requires a configured DB mirror",
        ));
    }
    if state.artifact_store.is_none() {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "credit estimate eval requires a configured artifact store",
        ));
    }
    match run_credit_estimate_eval(state.as_ref(), &query).await {
        Ok(response) => {
            tracing::info!(
                eval_target = query.target.label(),
                dry_run = response.dry_run,
                decisions = response.counts.decisions,
                submissions = response.counts.submissions,
                labelled = response.counts.labelled,
                unlabelled = response.counts.unlabelled,
                derived = response.counts.derived,
                failed = response.counts.failed,
                fit_passed = response.fit.as_ref().map(|fit| fit.passed),
                fit_tiers = response.fit.as_ref().map(|fit| fit.tiers),
                settlement_fit_tiers = response.settlement_fit.as_ref().map(|fit| fit.tiers),
                settlement_fit_table = response
                    .settlement_fit
                    .as_ref()
                    .map(|fit| fit.table.is_some()),
                "Trace Commons credit estimate eval completed"
            );
            Ok(Json(response))
        }
        Err(error) => {
            tracing::warn!(
                error_hash = %safe_display_error_hash(&error),
                "Trace Commons credit estimate eval failed"
            );
            Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "credit_estimate_eval_failed",
            ))
        }
    }
}
