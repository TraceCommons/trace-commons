//! Account-free local Insights entry point shared by native shells and CLI.
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::{LocalInsight, LocalInsightStore, SourceFormat, analyze_file};

/// Bound request bytes before parsing or reading caller-owned FFI memory.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalInsightsRequest {
    #[serde(default)]
    pub store_dir: Option<PathBuf>,
    pub operation: LocalInsightsOperation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalInsightsOperation {
    Analyze {
        source: SourceFormat,
        file: PathBuf,
        #[serde(default)]
        save: bool,
    },
    List,
    Explain {
        id: String,
    },
    Delete {
        id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalInsightsResponse {
    Analyze { insight: Box<LocalInsight> },
    List { insights: Vec<LocalInsight> },
    Explain { insight: Box<LocalInsight> },
    Delete { deleted: bool },
}

/// Resolve only local Insights storage; never resolve enrollment configuration.
pub fn open_store(store_dir: Option<&std::path::Path>) -> Result<LocalInsightStore> {
    let path = match store_dir {
        Some(path) => path.to_path_buf(),
        None => dirs::data_local_dir()
            .ok_or_else(|| anyhow!("insights-local-directory-unavailable"))?
            .join("trace-commons")
            .join("insights"),
    };
    LocalInsightStore::open(&path)
}

/// Synchronous local IO. Native callers must schedule this off the UI thread.
/// Dropping a UI task does not cancel a save/delete that has already started.
pub fn execute(request: LocalInsightsRequest) -> Result<LocalInsightsResponse> {
    let store = || open_store(request.store_dir.as_deref());
    Ok(match request.operation {
        LocalInsightsOperation::Analyze { source, file, save } => {
            let insight = if save {
                store()?.import(source, &file)?
            } else {
                analyze_file(source, &file)?
            };
            LocalInsightsResponse::Analyze {
                insight: Box::new(insight),
            }
        }
        LocalInsightsOperation::List => LocalInsightsResponse::List {
            insights: store()?.list()?,
        },
        LocalInsightsOperation::Explain { id } => LocalInsightsResponse::Explain {
            insight: Box::new(store()?.explain(&id)?),
        },
        LocalInsightsOperation::Delete { id } => LocalInsightsResponse::Delete {
            deleted: store()?.delete(&id)?,
        },
    })
}

/// Strict typed JSON boundary. Errors deliberately contain fixed labels only.
pub fn dispatch_json(bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_REQUEST_BYTES {
        bail!("insights-request-too-large");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow!("insights-request-invalid-utf8"))?;
    let request = serde_json::from_str(text).map_err(|_| anyhow!("insights-request-invalid"))?;
    let response = execute(request).map_err(|_| anyhow!("insights-operation-failed"))?;
    serde_json::to_string(&response).map_err(|_| anyhow!("insights-response-invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_untrusted_request_without_echoing_content() {
        for request in [
            br#"{"operation":{"type":"list","secret":"private"}}"#.as_slice(),
            br#"{"operation":{"type":"upload"}}"#,
            br#"{"operation":{"type":"list"},"secret":"private"}"#,
        ] {
            assert_eq!(
                dispatch_json(request).unwrap_err().to_string(),
                "insights-request-invalid"
            );
        }
        assert_eq!(
            dispatch_json(&[0xff]).unwrap_err().to_string(),
            "insights-request-invalid-utf8"
        );
        assert_eq!(
            dispatch_json(&vec![b' '; MAX_REQUEST_BYTES + 1])
                .unwrap_err()
                .to_string(),
            "insights-request-too-large"
        );
    }

    #[test]
    fn selected_file_lifecycle_keeps_unsaved_analysis_ephemeral() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        let store = temp.path().join("insights");
        std::fs::write(
            &file,
            concat!(
                "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n",
                "{\"role\":\"user\",\"content\":\"PRIVATE_BODY\",\"timestamp\":\"2026-09-11T10:00:00Z\"}\n"
            ),
        )
        .unwrap();
        let call = |operation| {
            execute(LocalInsightsRequest {
                store_dir: Some(store.clone()),
                operation,
            })
            .unwrap()
        };
        let LocalInsightsResponse::Analyze { insight } = call(LocalInsightsOperation::Analyze {
            source: SourceFormat::Trajectory,
            file: file.clone(),
            save: false,
        }) else {
            panic!("expected analysis")
        };
        assert!(!store.exists());
        assert!(
            !serde_json::to_string(&insight)
                .unwrap()
                .contains("PRIVATE_BODY")
        );
        call(LocalInsightsOperation::Analyze {
            source: SourceFormat::Trajectory,
            file: file.clone(),
            save: true,
        });
        let LocalInsightsResponse::Explain { insight: saved } =
            call(LocalInsightsOperation::Explain {
                id: insight.id.clone(),
            })
        else {
            panic!("expected explanation")
        };
        assert_eq!(saved.id, insight.id);
        assert!(matches!(
            call(LocalInsightsOperation::Delete { id: insight.id }),
            LocalInsightsResponse::Delete { deleted: true }
        ));
        assert!(
            matches!(call(LocalInsightsOperation::List), LocalInsightsResponse::List { insights } if insights.is_empty())
        );
        assert!(file.exists());
    }

    #[test]
    fn list_needs_only_an_explicit_local_directory() {
        let temp = tempfile::tempdir().unwrap();
        let request = LocalInsightsRequest {
            store_dir: Some(temp.path().join("insights")),
            operation: LocalInsightsOperation::List,
        };
        let response = dispatch_json(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap(),
            serde_json::json!({"type":"list","insights":[]})
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
