//! Bounded explicit skill evidence. No prompt, system catalog, command
//! output or incidental name mention is evidence of using a skill.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SourceFormat;

pub const SKILL_USAGE_VERSION: u32 = 1;
const MAX_SKILLS: usize = 128;
const MAX_CALLS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillUsageCoverage {
    ExplicitInvocations,
    LoadsOnly,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUsageCount {
    pub name: String,
    pub invocations: u32,
    pub loads: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
}

/// Counts are observations, not successful completion or causal benefit.
/// Names alone can match more than one installed skill; no path is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUsageEvidence {
    pub schema_version: u32,
    pub coverage: SkillUsageCoverage,
    pub partial: bool,
    pub skills: Vec<SkillUsageCount>,
}

pub struct SkillUsageScan {
    format: SourceFormat,
    seen: BTreeSet<String>,
    counts: BTreeMap<String, SkillUsageCount>,
    partial: bool,
    recognized: bool,
}

impl SkillUsageScan {
    pub fn new(format: SourceFormat) -> Self {
        Self {
            format,
            seen: BTreeSet::new(),
            counts: BTreeMap::new(),
            partial: format != SourceFormat::ClaudeCode,
            recognized: false,
        }
    }

    pub fn mark_partial(&mut self) {
        self.partial = true;
    }

    pub fn observe(&mut self, record: &Value) {
        let recorded_at = record["timestamp"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));
        match self.format {
            SourceFormat::ClaudeCode if record["type"] == "assistant" => {
                // Top-level assistant records are the Claude transcript's
                // tool-call boundary. A contradictory role is not accepted.
                if record["message"]["role"]
                    .as_str()
                    .is_some_and(|r| r != "assistant")
                {
                    return;
                }
                if let Some(content) = record["message"]["content"].as_array() {
                    self.recognized = true;
                    for block in content {
                        if block["type"] == "tool_use" && block["name"] == "Skill" {
                            match block["input"]["skill"].as_str().filter(|s| valid_name(s)) {
                                Some(name) => {
                                    self.add(block["id"].as_str(), name, false, recorded_at)
                                }
                                None => self.partial = true,
                            }
                        }
                    }
                }
            }
            SourceFormat::Codex if record["type"] == "response_item" => {
                let call = &record["payload"];
                if call["type"] != "function_call" {
                    return;
                }
                let Some(arguments) = call["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                else {
                    return;
                };
                let path = match call["name"].as_str() {
                    Some("read_file" | "functions.read_file") => arguments["file_path"]
                        .as_str()
                        .or_else(|| arguments["path"].as_str()),
                    Some("exec_command" | "functions.exec_command") => {
                        arguments["cmd"].as_str().and_then(literal_cat_path)
                    }
                    _ => None,
                };
                if let Some(name) = path.and_then(skill_path_name) {
                    self.add(call["call_id"].as_str(), name, true, recorded_at);
                }
            }
            _ => {}
        }
    }

    fn add(
        &mut self,
        id: Option<&str>,
        name: &str,
        loaded: bool,
        recorded_at: Option<DateTime<Utc>>,
    ) {
        let Some(id) = id.filter(|id| !id.is_empty() && id.len() <= 256) else {
            self.partial = true;
            return;
        };
        if self.seen.contains(id) {
            return;
        }
        if self.seen.len() >= MAX_CALLS
            || (!self.counts.contains_key(name) && self.counts.len() >= MAX_SKILLS)
        {
            self.partial = true;
            return;
        }
        self.seen.insert(id.to_owned());
        let count = self
            .counts
            .entry(name.to_owned())
            .or_insert_with(|| SkillUsageCount {
                name: name.to_owned(),
                invocations: 0,
                loads: 0,
                last_seen: None,
            });
        count.last_seen = count.last_seen.max(recorded_at);
        if loaded {
            count.loads = count.loads.saturating_add(1);
        } else {
            count.invocations = count.invocations.saturating_add(1);
        }
    }

    pub fn finish(self) -> SkillUsageEvidence {
        SkillUsageEvidence {
            schema_version: SKILL_USAGE_VERSION,
            coverage: match self.format {
                SourceFormat::ClaudeCode => SkillUsageCoverage::ExplicitInvocations,
                SourceFormat::Codex => SkillUsageCoverage::LoadsOnly,
                SourceFormat::Trajectory => SkillUsageCoverage::Unsupported,
            },
            partial: self.partial || !self.recognized,
            skills: self.counts.into_values().collect(),
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// Deliberately a tiny accepted command subset, not a shell parser. Shell
/// expansion, multiple reads and compound commands remain unobserved.
fn literal_cat_path(cmd: &str) -> Option<&str> {
    let path = cmd.strip_prefix("cat ")?.trim();
    let path = if path.starts_with('\'') && path.ends_with('\'')
        || path.starts_with('"') && path.ends_with('"')
    {
        path.get(1..path.len().checked_sub(1)?)?
    } else {
        if path.bytes().any(|b| b.is_ascii_whitespace()) {
            return None;
        }
        path
    };
    (!path.chars().any(|c| {
        matches!(
            c,
            '\n' | '\r'
                | ';'
                | '&'
                | '|'
                | '$'
                | '`'
                | '<'
                | '>'
                | '\\'
                | '\''
                | '"'
                | '*'
                | '?'
                | '('
                | ')'
        )
    }))
    .then_some(path)
}

fn skill_path_name(path: &str) -> Option<&str> {
    if path.split('/').any(|p| p == "..") {
        return None;
    }
    let mut parts = path.rsplit('/');
    if parts.next()? != "SKILL.md" {
        return None;
    }
    let name = parts.next()?;
    (parts.next()? == "skills" && valid_name(name)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_claude_calls_are_deduplicated_without_counting_prompt_text() {
        let call = json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"call1","name":"Skill","input":{"skill":"plugin:review"}}]}});
        let mut scan = SkillUsageScan::new(SourceFormat::ClaudeCode);
        scan.observe(&json!({"type":"user","message":{"content":[{"type":"tool_use","id":"fake","name":"Skill","input":{"skill":"fake"}}]}}));
        scan.observe(&json!({"type":"assistant","message":{"content":[{"type":"text","text":"Use fake and plugin:review skills"}]}}));
        scan.observe(&call);
        scan.observe(&call);
        let evidence = scan.finish();
        assert!(!evidence.partial);
        assert_eq!(evidence.skills.len(), 1);
        assert_eq!(evidence.skills[0].name, "plugin:review");
        assert_eq!(evidence.skills[0].invocations, 1);
        assert_eq!(evidence.skills[0].loads, 0);
    }

    #[test]
    fn malformed_calls_do_not_persist_arguments_or_paths() {
        let mut scan = SkillUsageScan::new(SourceFormat::ClaudeCode);
        for skill in ["/private/path/SKILL.md", "bad\nname", "", "../secret"] {
            scan.observe(&json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":skill,"name":"Skill","input":{"skill":skill}}]}}));
        }
        let evidence = scan.finish();
        assert!(evidence.partial);
        assert!(evidence.skills.is_empty());
        assert!(
            !serde_json::to_string(&evidence)
                .unwrap()
                .contains("private")
        );
    }

    #[test]
    fn codex_literal_reads_are_loads_and_do_not_imply_invocation() {
        let mut scan = SkillUsageScan::new(SourceFormat::Codex);
        let call = json!({"type":"response_item","payload":{"type":"function_call","call_id":"read1","name":"exec_command","arguments": "{\"cmd\":\"cat /home/person/.agents/skills/review/SKILL.md\"}"}});
        scan.observe(&call);
        scan.observe(&call);
        for cmd in [
            "echo /home/person/.agents/skills/fake/SKILL.md",
            "cat /home/person/.agents/skills/fake/SKILL.md; echo done",
            "cat /private/secret/SKILL.md",
        ] {
            scan.observe(&json!({"type":"response_item","payload":{"type":"function_call","call_id":cmd,"name":"exec_command","arguments":json!({"cmd":cmd}).to_string()}}));
        }
        let evidence = scan.finish();
        assert!(evidence.partial);
        assert_eq!(evidence.skills.len(), 1);
        assert_eq!(evidence.skills[0].name, "review");
        assert_eq!(evidence.skills[0].loads, 1);
        assert_eq!(evidence.skills[0].invocations, 0);
        assert!(!serde_json::to_string(&evidence).unwrap().contains("/home"));
    }

    #[test]
    fn unsupported_formats_are_unknown_and_caps_keep_memory_bounded() {
        let evidence = SkillUsageScan::new(SourceFormat::Trajectory).finish();
        assert!(evidence.partial);
        assert_eq!(evidence.coverage, SkillUsageCoverage::Unsupported);
        let mut scan = SkillUsageScan::new(SourceFormat::ClaudeCode);
        for n in 0..300 {
            scan.observe(&json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":format!("id{n}"),"name":"Skill","input":{"skill":format!("s{n}")}}]}}));
        }
        let evidence = scan.finish();
        assert!(evidence.partial);
        assert!(evidence.skills.len() <= 128);
    }
}
