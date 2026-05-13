use std::path::{Path, PathBuf};

use serde::Deserialize;

// Incumbent ids whose license is grandfathered. Update this when the
// production perplexity scorer changes; see
// docs/superpowers/specs/2026-05-13-model-bakeoff-retrofit-design.md.
const INCUMBENT_CANDIDATES: &[&str] = &["llama-3.1-8b-instruct"];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum CandidateLicense {
    #[serde(rename = "apache-2.0")]
    Apache2,
    #[serde(rename = "mit")]
    Mit,
    #[serde(rename = "llama-community")]
    LlamaCommunity,
    #[serde(rename = "gemma-custom")]
    GemmaCustom,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateArch {
    Llama,
    Qwen2,
    Gemma3,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub path: PathBuf,
    pub arch: CandidateArch,
    pub license: CandidateLicense,
    #[serde(default)]
    pub params_b: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub candidate: Vec<Candidate>,
}

#[derive(Debug, Clone)]
pub struct ValidatedManifest {
    pub candidates: Vec<Candidate>,
    pub warnings: Vec<String>,
}

impl ValidatedManifest {
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn parse_manifest_str(raw: &str) -> anyhow::Result<ValidatedManifest> {
    let manifest: Manifest =
        toml::from_str(raw).map_err(|e| anyhow::anyhow!("manifest parse error: {e}"))?;
    if manifest.candidate.is_empty() {
        anyhow::bail!("manifest must contain at least one candidate");
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &manifest.candidate {
        if !seen.insert(c.id.clone()) {
            anyhow::bail!("duplicate candidate id: {}", c.id);
        }
    }
    let mut warnings = Vec::new();
    for c in &manifest.candidate {
        if matches!(c.license, CandidateLicense::LlamaCommunity)
            && !INCUMBENT_CANDIDATES.contains(&c.id.as_str())
        {
            warnings.push(format!(
                "candidate {} uses non-permissive license llama-community; \
                 only Apache-2.0 or MIT are accepted for new picks",
                c.id
            ));
        }
    }
    Ok(ValidatedManifest {
        candidates: manifest.candidate,
        warnings,
    })
}

pub fn parse_manifest_file(path: &Path) -> anyhow::Result<ValidatedManifest> {
    let raw = std::fs::read_to_string(path)?;
    parse_manifest_str(&raw)
}
