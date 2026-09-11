//! INTEGRATION: discovers an eligible NEAR AI model and bounds every catalog
//! and chat-completion response used by the skill comparison.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::skill_loop::evaluation::fixtures::{ApplicabilityFixture, EvaluationFixture};
use crate::skill_loop::evaluation::scoring::{
    applicability_prompt, applicability_system_prompt, fixture_prompt, plan_system_prompt,
};
use crate::skill_loop::evaluation::{
    EvaluationArm, EvaluationUsage, OUTPUT_TOKEN_LIMIT, REQUEST_TIMEOUT_SECS, SkillEvaluationError,
};

const CLOUD_API_BASE: &str = "https://cloud-api.near.ai/v1";
const MAX_COMPLETION_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_RETAINED_RAW_OUTPUT_BYTES: usize = 8 * 1024;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const CATALOG_PAGE_LIMIT: usize = 100;
const MAX_CATALOG_MODELS: usize = 512;
const MAX_CATALOG_PAGES: usize = MAX_CATALOG_MODELS.div_ceil(CATALOG_PAGE_LIMIT);

#[derive(Clone)]
pub(super) struct NearAiEvaluationClient {
    http: Client,
    base_url: Arc<str>,
    api_key: Arc<str>,
    model: Arc<str>,
}

pub(super) struct ModelCompletion {
    pub(super) raw_output: String,
    pub(super) request_id: Option<String>,
    pub(super) served_model: String,
    pub(super) finish_reason: String,
    pub(super) usage: EvaluationUsage,
}

#[derive(Deserialize)]
struct ModelCatalog {
    #[serde(default)]
    models: Option<Vec<PricedModelRecord>>,
    #[serde(default)]
    data: Option<Vec<LegacyModelRecord>>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    total: Option<i64>,
}

struct ModelCatalogPage {
    models: Vec<ModelRecord>,
    limit: usize,
    offset: usize,
    total: usize,
}

enum ParsedModelCatalog {
    Current(ModelCatalogPage),
    Legacy(Vec<ModelRecord>),
}

#[derive(Deserialize)]
struct PricedModelRecord {
    #[serde(rename = "modelId")]
    model_id: String,
    metadata: PricedModelMetadata,
}

#[derive(Deserialize)]
struct PricedModelMetadata {
    #[serde(rename = "ownedBy")]
    owned_by: String,
    #[serde(rename = "isReady", default)]
    is_ready: Option<bool>,
    #[serde(rename = "supportedFeatures", default)]
    supported_features: Option<Vec<String>>,
    #[serde(default)]
    architecture: Option<ModelArchitecture>,
}

#[derive(Deserialize)]
struct LegacyModelRecord {
    id: String,
    owned_by: String,
    #[serde(default)]
    is_ready: Option<bool>,
    supported_features: Vec<String>,
    #[serde(default)]
    output_modalities: Option<Vec<String>>,
    #[serde(default)]
    architecture: Option<ModelArchitecture>,
}

#[derive(Deserialize)]
struct ModelArchitecture {
    #[serde(rename = "outputModalities")]
    output_modalities: Vec<String>,
}

struct ModelRecord {
    id: String,
    owned_by: String,
    is_ready: Option<bool>,
    supported_features: Vec<String>,
    output_modalities: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ChatCompletion {
    model: String,
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    reasoning_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

impl NearAiEvaluationClient {
    pub(super) async fn discover(api_key: String) -> Result<Self, SkillEvaluationError> {
        Self::discover_at(CLOUD_API_BASE.to_string(), api_key, true).await
    }

    async fn discover_at(
        base_url: String,
        api_key: String,
        https_only: bool,
    ) -> Result<Self, SkillEvaluationError> {
        if api_key.trim().is_empty() {
            return Err(SkillEvaluationError::CredentialRequired);
        }
        let http = build_client(https_only, REQUEST_TIMEOUT_SECS)?;
        let catalog = fetch_model_catalog(&http, &base_url, &api_key).await?;
        let model = choose_private_model(&catalog)
            .ok_or(SkillEvaluationError::NoPrivateModel)?
            .to_string();
        Ok(Self {
            http,
            base_url: Arc::from(base_url),
            api_key: Arc::from(api_key),
            model: Arc::from(model),
        })
    }

    #[cfg(test)]
    pub(super) fn at(
        base_url: String,
        api_key: String,
        model: String,
    ) -> Result<Self, SkillEvaluationError> {
        let http = build_client(false, 5)?;
        Ok(Self {
            http,
            base_url: Arc::from(base_url),
            api_key: Arc::from(api_key),
            model: Arc::from(model),
        })
    }

    pub(super) fn model(&self) -> &str {
        self.model.as_ref()
    }

    pub(super) async fn complete(
        &self,
        fixture: EvaluationFixture,
        arm: EvaluationArm,
        skill_name: &str,
        skill_md: &str,
    ) -> Result<ModelCompletion, SkillEvaluationError> {
        self.complete_prompts(
            plan_system_prompt(),
            fixture_prompt(fixture, arm, skill_name, skill_md),
        )
        .await
    }

    pub(super) async fn complete_applicability(
        &self,
        fixture: ApplicabilityFixture,
        arm: EvaluationArm,
        skill_name: &str,
        skill_description: &str,
    ) -> Result<ModelCompletion, SkillEvaluationError> {
        self.complete_prompts(
            applicability_system_prompt(),
            applicability_prompt(fixture, arm, skill_name, skill_description),
        )
        .await
    }

    async fn complete_prompts(
        &self,
        system_prompt: &str,
        user_prompt: String,
    ) -> Result<ModelCompletion, SkillEvaluationError> {
        let body = serde_json::json!({
            "model": self.model.as_ref(),
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt,
                },
                {
                    "role": "user",
                    "content": user_prompt,
                }
            ],
            "max_tokens": OUTPUT_TOKEN_LIMIT,
            "temperature": 0,
            "response_format": {"type": "json_object"},
            "reasoning": {"enabled": false}
        });
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(self.api_key.as_ref())
            .json(&body)
            .send()
            .await
            .map_err(|_| SkillEvaluationError::RequestUnavailable)?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status {
                StatusCode::PAYMENT_REQUIRED => SkillEvaluationError::FundingRequired,
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    SkillEvaluationError::ProviderRejected
                }
                _ => SkillEvaluationError::RequestUnavailable,
            });
        }
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 128 && value.is_ascii())
            .map(str::to_string);
        let bytes = read_bounded(
            response,
            MAX_COMPLETION_RESPONSE_BYTES,
            SkillEvaluationError::RequestUnavailable,
        )
        .await?;
        let completion: ChatCompletion =
            serde_json::from_slice(&bytes).map_err(|_| SkillEvaluationError::RequestUnavailable)?;
        let choice = completion
            .choices
            .into_iter()
            .next()
            .ok_or(SkillEvaluationError::RequestUnavailable)?;
        let raw_output = choice.message.content.unwrap_or_default();
        if raw_output.len() > MAX_RETAINED_RAW_OUTPUT_BYTES {
            return Err(SkillEvaluationError::ResponseTooLarge);
        }
        let usage = completion.usage.unwrap_or(ChatUsage {
            prompt_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            total_tokens: None,
        });
        Ok(ModelCompletion {
            raw_output,
            request_id,
            served_model: completion.model,
            finish_reason: choice
                .finish_reason
                .unwrap_or_else(|| "unknown".to_string()),
            usage: EvaluationUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                total_tokens: usage.total_tokens,
            },
        })
    }
}

fn build_client(https_only: bool, timeout_secs: u64) -> Result<Client, SkillEvaluationError> {
    Client::builder()
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(timeout_secs.min(10)))
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|_| SkillEvaluationError::ClientUnavailable)
}

async fn fetch_model_catalog(
    http: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelRecord>, SkillEvaluationError> {
    let first_response = send_catalog_request(
        http,
        format!("{base_url}/model/list"),
        api_key,
        Some((CATALOG_PAGE_LIMIT, 0)),
    )
    .await?;
    let Some(first_response) = first_response else {
        return fetch_legacy_catalog(http, base_url, api_key).await;
    };

    let mut response = Some(first_response);
    let mut byte_count = 0_usize;
    let mut expected_total = None;
    let mut next_offset = 0_usize;
    let mut models = Vec::new();
    let mut model_ids = HashSet::new();

    for _ in 0..MAX_CATALOG_PAGES {
        let page_response = match response.take() {
            Some(response) => response,
            None => send_catalog_request(
                http,
                format!("{base_url}/model/list"),
                api_key,
                Some((CATALOG_PAGE_LIMIT, next_offset)),
            )
            .await?
            .ok_or(SkillEvaluationError::CatalogUnavailable)?,
        };
        let remaining_bytes = MAX_CATALOG_BYTES.saturating_sub(byte_count);
        let bytes = read_bounded(
            page_response,
            remaining_bytes,
            SkillEvaluationError::CatalogUnavailable,
        )
        .await?;
        byte_count = byte_count.saturating_add(bytes.len());
        match parse_model_catalog(&bytes)? {
            ParsedModelCatalog::Legacy(legacy) if next_offset == 0 => {
                return bounded_legacy_models(legacy);
            }
            ParsedModelCatalog::Legacy(_) => {
                return Err(SkillEvaluationError::CatalogUnavailable);
            }
            ParsedModelCatalog::Current(page) => {
                if page.limit != CATALOG_PAGE_LIMIT || page.offset != next_offset {
                    return Err(SkillEvaluationError::CatalogUnavailable);
                }
                if page.total > MAX_CATALOG_MODELS {
                    return Err(SkillEvaluationError::ResponseTooLarge);
                }
                if expected_total.is_some_and(|total| total != page.total) {
                    return Err(SkillEvaluationError::CatalogUnavailable);
                }
                expected_total = Some(page.total);
                let expected_page_size = CATALOG_PAGE_LIMIT.min(page.total - page.offset);
                if page.models.len() != expected_page_size {
                    return Err(SkillEvaluationError::CatalogUnavailable);
                }
                for model in page.models {
                    if !model_ids.insert(model.id.clone()) {
                        return Err(SkillEvaluationError::CatalogUnavailable);
                    }
                    models.push(model);
                }
                next_offset = models.len();
                if next_offset == page.total {
                    return Ok(models);
                }
            }
        }
    }

    Err(SkillEvaluationError::CatalogUnavailable)
}

async fn fetch_legacy_catalog(
    http: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelRecord>, SkillEvaluationError> {
    let response = send_catalog_request(http, format!("{base_url}/models"), api_key, None)
        .await?
        .ok_or(SkillEvaluationError::CatalogUnavailable)?;
    let bytes = read_bounded(
        response,
        MAX_CATALOG_BYTES,
        SkillEvaluationError::CatalogUnavailable,
    )
    .await?;
    match parse_model_catalog(&bytes)? {
        ParsedModelCatalog::Legacy(models) => bounded_legacy_models(models),
        ParsedModelCatalog::Current(_) => Err(SkillEvaluationError::CatalogUnavailable),
    }
}

async fn send_catalog_request(
    http: &Client,
    url: String,
    api_key: &str,
    pagination: Option<(usize, usize)>,
) -> Result<Option<reqwest::Response>, SkillEvaluationError> {
    let mut request = http.get(url).bearer_auth(api_key);
    if let Some((limit, offset)) = pagination {
        request = request.query(&[("limit", limit), ("offset", offset)]);
    }
    let response = request
        .send()
        .await
        .map_err(|_| SkillEvaluationError::CatalogUnavailable)?;
    let status = response.status();
    if status.is_success() {
        return Ok(Some(response));
    }
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) {
        return Ok(None);
    }
    Err(match status {
        StatusCode::PAYMENT_REQUIRED => SkillEvaluationError::FundingRequired,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SkillEvaluationError::ProviderRejected,
        _ => SkillEvaluationError::CatalogUnavailable,
    })
}

fn parse_model_catalog(bytes: &[u8]) -> Result<ParsedModelCatalog, SkillEvaluationError> {
    let catalog: ModelCatalog =
        serde_json::from_slice(bytes).map_err(|_| SkillEvaluationError::CatalogUnavailable)?;
    match (catalog.models, catalog.data) {
        (Some(models), None) => {
            let limit = parse_catalog_number(catalog.limit)?;
            let offset = parse_catalog_number(catalog.offset)?;
            let total = parse_catalog_number(catalog.total)?;
            if offset > total {
                return Err(SkillEvaluationError::CatalogUnavailable);
            }
            Ok(ParsedModelCatalog::Current(ModelCatalogPage {
                models: models.into_iter().map(ModelRecord::from).collect(),
                limit,
                offset,
                total,
            }))
        }
        (None, Some(models)) => Ok(ParsedModelCatalog::Legacy(
            models.into_iter().map(ModelRecord::from).collect(),
        )),
        _ => Err(SkillEvaluationError::CatalogUnavailable),
    }
}

fn parse_catalog_number(value: Option<i64>) -> Result<usize, SkillEvaluationError> {
    value
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(SkillEvaluationError::CatalogUnavailable)
}

fn bounded_legacy_models(
    models: Vec<ModelRecord>,
) -> Result<Vec<ModelRecord>, SkillEvaluationError> {
    if models.len() > MAX_CATALOG_MODELS {
        return Err(SkillEvaluationError::ResponseTooLarge);
    }
    Ok(models)
}

impl From<PricedModelRecord> for ModelRecord {
    fn from(model: PricedModelRecord) -> Self {
        Self {
            id: model.model_id,
            owned_by: model.metadata.owned_by,
            is_ready: model.metadata.is_ready,
            supported_features: model.metadata.supported_features.unwrap_or_default(),
            output_modalities: model
                .metadata
                .architecture
                .map(|architecture| architecture.output_modalities),
        }
    }
}

impl From<LegacyModelRecord> for ModelRecord {
    fn from(model: LegacyModelRecord) -> Self {
        let architecture_modalities = model
            .architecture
            .map(|architecture| architecture.output_modalities);
        let output_modalities = match (model.output_modalities, architecture_modalities) {
            (Some(flat), Some(nested)) if flat != nested => None,
            (Some(flat), _) => Some(flat),
            (None, nested) => nested,
        };
        Self {
            id: model.id,
            owned_by: model.owned_by,
            is_ready: model.is_ready,
            supported_features: model.supported_features,
            output_modalities,
        }
    }
}

fn choose_private_model(models: &[ModelRecord]) -> Option<&str> {
    // Selection is a deterministic compatibility policy, not an implicit
    // quality score. Prefer known NEAR-owned text models whose structured
    // output contract has been exercised by these fixtures, then use lexical
    // order for a newly eligible model. One discovered model remains pinned
    // for every arm in the comparison so catalog movement cannot bias it.
    // Changing this order requires rerunning the transport and scoring suites.
    const PREFERRED: &[&str] = &[
        "deepseek-ai/DeepSeek-V4-Flash",
        "z-ai/glm-5.3-flash",
        "z-ai/glm-5.2",
        "Qwen/Qwen3.6-35B-A3B-FP8",
        "Qwen/Qwen3.8-27B",
        "zai-org/GLM-5.1-FP8",
    ];
    let eligible = |model: &&ModelRecord| {
        model.owned_by == "nearai"
            && model.is_ready == Some(true)
            && model
                .supported_features
                .iter()
                .any(|feature| matches!(feature.as_str(), "json_mode" | "structured_outputs"))
            && model
                .output_modalities
                .as_deref()
                .is_some_and(|modalities| modalities.iter().any(|value| value == "text"))
            && !["privacy", "whisper", "embedding", "reranker", "flux", "vl-"]
                .iter()
                .any(|blocked| model.id.to_ascii_lowercase().contains(blocked))
    };
    let eligible_models = models.iter().filter(eligible).collect::<Vec<_>>();
    for preferred in PREFERRED {
        if eligible_models.iter().any(|model| model.id == *preferred) {
            return Some(preferred);
        }
    }
    eligible_models
        .into_iter()
        .map(|model| model.id.as_str())
        .min()
}

async fn read_bounded(
    mut response: reqwest::Response,
    maximum: usize,
    read_error: SkillEvaluationError,
) -> Result<Vec<u8>, SkillEvaluationError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(SkillEvaluationError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| read_error)? {
        if body.len().saturating_add(chunk.len()) > maximum {
            return Err(SkillEvaluationError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests;
