//! Bounded account-free mission draft inbox service shared by local clients.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::mission_draft::{
    MissionDraftDelete, MissionDraftImport, MissionDraftInbox, MissionDraftSummary,
    StoredMissionDraft,
};

pub const MAX_MISSION_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_MISSION_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MissionDraftRequest {
    #[serde(default)]
    pub store_dir: Option<PathBuf>,
    pub operation: MissionDraftOperation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionDraftOperation {
    Import { file: PathBuf },
    List {},
    Show { id: String },
    Delete { id: String },
    Copy {},
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MissionDraftResponse {
    Import { draft: MissionDraftImport },
    List { drafts: Vec<MissionDraftSummary> },
    Show { draft: Box<StoredMissionDraft> },
    Delete { draft: MissionDraftDelete },
    Copy { copy: BTreeMap<String, String> },
}

/// Shared presentation vocabulary. Proposal content is never interpolated.
pub fn ui_copy() -> BTreeMap<String, String> {
    [
        ("title", "Mission drafts"),
        ("intro", "Review mission proposals saved on this device."),
        ("empty", "No local mission drafts."),
        ("choose_file", "Choose proposal file"),
        ("import", "Import draft"),
        ("refresh", "Refresh drafts"),
        ("show", "Inspect draft"),
        ("delete", "Delete local draft"),
        ("delete_confirm_title", "Delete local mission draft?"),
        ("delete_confirm", "Delete this draft from the local inbox? The selected source file will remain."),
        ("added", "Draft added to the local inbox."),
        ("duplicate", "This draft is already in the local inbox."),
        ("deleted", "Deleted the local mission draft. The selected source file remains."),
        ("proposal_sha256", "Proposal SHA-256"),
        ("source_count", "Declared source count"),
        ("proposal_title", "Proposed title"),
        ("source_claim", "Claim to test"),
        ("task", "Proposed task"),
        ("starting_artifact", "Starting artifact"),
        ("starting_artifact_digest", "Starting artifact SHA-256"),
        ("source_urls", "Declared source URLs (plain text)"),
        ("success_criteria", "Proposed success criteria"),
        ("required_evidence", "Required evidence"),
        ("allowed_models", "Allowed models"),
        ("allowed_tools", "Allowed tools"),
        ("proposed_budget", "Proposed budget"),
        ("duration_seconds", "Maximum duration (seconds)"),
        ("input_tokens", "Maximum input tokens"),
        ("output_tokens", "Maximum output tokens"),
        ("author_unverified", "Proposed author (unverified)"),
        ("evaluator_unverified", "Proposed evaluator (unverified)"),
        ("rubric_version", "Proposed rubric version"),
        ("needs_curator_review", "Needs curator review"),
        ("review_notice", "Sources and claims have not been verified."),
        ("authority_notice", "Curator review is required before publication or participation."),
        ("display_notice", "Imported proposal details are shown as supplied."),
        ("working", "Updating local mission drafts…"),
        ("cancel", "Cancel"),
        ("close", "Close"),
        ("error", "The local mission draft operation could not be completed. Check the selected file or draft and try again."),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

/// Synchronous local IO. Native callers schedule this off the UI thread.
pub fn execute(request: MissionDraftRequest) -> Result<MissionDraftResponse> {
    if matches!(request.operation, MissionDraftOperation::Copy {}) {
        let response = MissionDraftResponse::Copy { copy: ui_copy() };
        response_json(&response)?;
        return Ok(response);
    }
    let inbox = MissionDraftInbox::resolve(request.store_dir.as_deref())?;
    let response = match request.operation {
        MissionDraftOperation::Import { file } => MissionDraftResponse::Import {
            draft: inbox.import_file(&file)?,
        },
        MissionDraftOperation::List {} => MissionDraftResponse::List {
            drafts: inbox.list()?,
        },
        MissionDraftOperation::Show { id } => MissionDraftResponse::Show {
            draft: Box::new(inbox.show(&id)?),
        },
        MissionDraftOperation::Delete { id } => MissionDraftResponse::Delete {
            draft: inbox.delete(&id)?,
        },
        MissionDraftOperation::Copy {} => unreachable!("copy returned before inbox resolution"),
    };
    response_json(&response)?;
    Ok(response)
}

pub fn dispatch_json(bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_MISSION_REQUEST_BYTES {
        return Err(anyhow!("mission-draft-request-too-large"));
    }
    std::str::from_utf8(bytes).map_err(|_| anyhow!("mission-draft-request-invalid-utf8"))?;
    let request: MissionDraftRequest =
        serde_json::from_slice(bytes).map_err(|_| anyhow!("mission-draft-request-invalid"))?;
    let response = execute(request).map_err(safe_error)?;
    response_json(&response)
}

fn safe_error(error: anyhow::Error) -> anyhow::Error {
    let label = error.to_string();
    if matches!(
        label.as_str(),
        "mission-draft-too-large"
            | "mission-draft-invalid-json"
            | "mission-draft-version-unsupported"
            | "mission-draft-field-invalid"
            | "mission-draft-url-invalid"
            | "mission-draft-budget-invalid"
            | "mission-draft-file-unreadable"
            | "mission-draft-file-not-regular"
            | "mission-draft-store-unavailable"
            | "mission-draft-store-busy"
            | "mission-draft-store-unreadable"
            | "mission-draft-store-invalid"
            | "mission-draft-store-version-unsupported"
            | "mission-draft-store-full"
            | "mission-draft-store-write-failed"
            | "mission-draft-store-requires-private-directory"
            | "mission-draft-not-found"
            | "mission-draft-response-too-large"
    ) {
        anyhow!(label)
    } else {
        anyhow!("mission-draft-operation-failed")
    }
}

fn response_json(response: &MissionDraftResponse) -> Result<String> {
    struct BoundedWriter {
        bytes: Vec<u8>,
    }
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_MISSION_RESPONSE_BYTES.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("bounded mission response"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter { bytes: Vec::new() };
    serde_json::to_writer(&mut writer, response)
        .map_err(|_| anyhow!("mission-draft-response-too-large"))?;
    String::from_utf8(writer.bytes).map_err(|_| anyhow!("mission-draft-operation-failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::Path;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../trace-commons-protocol/tests/fixtures/mission-draft.json")
    }

    fn call(store: &Path, operation: Value) -> Result<Value> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "store_dir": store,
            "operation": operation
        }))?;
        Ok(serde_json::from_str(&dispatch_json(&bytes)?)?)
    }

    #[test]
    fn list_and_copy_do_not_create_an_absent_store() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("inbox");
        assert_eq!(
            call(&store, serde_json::json!({"type":"list"})).unwrap(),
            serde_json::json!({"type":"list","drafts":[]})
        );
        assert!(call(&store, serde_json::json!({"type":"copy"})).is_ok());
        assert!(!store.exists());
    }

    #[test]
    fn import_and_list_hide_body_while_explicit_show_returns_it() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("inbox");
        let imported = call(
            &store,
            serde_json::json!({"type":"import","file":fixture()}),
        )
        .unwrap();
        let id = imported["draft"]["id"].as_str().unwrap();
        assert!(imported["draft"].get("proposal").is_none());
        let listed = call(&store, serde_json::json!({"type":"list"})).unwrap();
        assert!(listed["drafts"][0].get("proposal").is_none());
        let shown = call(&store, serde_json::json!({"type":"show","id":id})).unwrap();
        assert!(shown["draft"]["proposal"]["task"].is_string());
    }

    #[test]
    fn malformed_unknown_and_oversized_requests_are_fixed_errors() {
        assert_eq!(
            dispatch_json(&[0xff]).unwrap_err().to_string(),
            "mission-draft-request-invalid-utf8"
        );
        assert_eq!(
            dispatch_json(br#"{"operation":{"type":"list","extra":true}}"#)
                .unwrap_err()
                .to_string(),
            "mission-draft-request-invalid"
        );
        assert_eq!(
            dispatch_json(&vec![b' '; MAX_MISSION_REQUEST_BYTES + 1])
                .unwrap_err()
                .to_string(),
            "mission-draft-request-too-large"
        );
    }
}
