//! Talks to the Antigravity language server API discovered by
//! [`super::endpoint::discover`]: lists the operator's trajectories and
//! fetches one trajectory's steps.
//!
//! The identifier trap: a conversation's FILE NAME is its *cascade* id; the
//! `trajectoryId` recorded inside the conversation is a *different* uuid.
//! `GetCascadeTrajectorySteps` takes `{"cascadeId": "..."}` -- sending
//! `trajectoryId`, or the wrong uuid under either name, returns the same
//! generic "trajectory not found" an empty request produces, so a wrong
//! identifier here is not self-diagnosing.
//!
//! The listing call is `GetAllCascadeTrajectories`, not
//! `GetUserTrajectoryDescriptions` -- the latter lists a different concept
//! ("user trajectories") whose `trajectoryId` cannot fetch steps under any
//! field name; a live probe confirmed it always answers "trajectory not
//! found", the same generic failure an empty request produces.
//! `GetAllCascadeTrajectories` is keyed by cascade id -- exactly the
//! identifier `GetCascadeTrajectorySteps` takes -- so listing then fetching
//! needs no id-mapping step at all.
//!
//! Every failure this module can hit -- transport, non-200, malformed JSON
//! -- collapses to [`ERR_API_FAILED`]. A `reqwest` error's `Display` can
//! carry the request URL, and the URL carries the port; that, and the CSRF
//! token, must never reach an error, a log line, or a panic, so `reqwest`
//! errors are discarded rather than formatted, matching `endpoint.rs`.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

use super::endpoint::Endpoint;

/// The RPC service path every method here hangs off of.
const SERVICE_PATH: &str = "/exa.language_server_pb.LanguageServerService";
const LIST_METHOD: &str = "GetAllCascadeTrajectories";
const STEPS_METHOD: &str = "GetCascadeTrajectorySteps";
const CSRF_HEADER: &str = "x-codeium-csrf-token";

/// Per-request timeout. Fetching steps for a large trajectory can take
/// longer than the endpoint probe's 250ms window, so this is generous
/// rather than tight -- matching `IssuerClient::new`'s 30s.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const ERR_API_FAILED: &str = "antigravity-api-failed";

/// One entry from `GetAllCascadeTrajectories`. `cascade_id` is the map key
/// the response is keyed by -- the identifier `GetCascadeTrajectorySteps`
/// actually takes -- so it is always present, unlike `trajectory_id`'s
/// counterpart under the abandoned `GetUserTrajectoryDescriptions` listing.
pub(crate) struct TrajectoryDescription {
    pub cascade_id: String,
    pub trajectory_id: String,
    pub workspace_uri: Option<String>,
    pub git_root: Option<String>,
    pub git_branch: Option<String>,
    pub summary: Option<String>,
    pub step_count: Option<u32>,
}

/// A trait so `convert` and the command are testable against recorded
/// responses with no IDE running. Only [`HttpApi`] touches the network.
pub(crate) trait AntigravityApi {
    async fn list_trajectories(&self) -> Result<Vec<TrajectoryDescription>>;
    async fn fetch_steps(&self, cascade_id: &str) -> Result<serde_json::Value>;
}

#[derive(Deserialize, Default)]
struct RawListing {
    #[serde(default, rename = "trajectorySummaries")]
    trajectory_summaries: BTreeMap<String, RawSummary>,
}

/// Proto3 JSON omits any field left at its default value -- an empty
/// string, a zero number, `false`, an empty list -- rather than sending it
/// explicitly. Every non-`Option` field here therefore needs
/// `#[serde(default)]` even though it "must" be present: a listing entry
/// that happens to carry an empty `trajectoryId` (or any other
/// default-valued field this struct grows) would otherwise fail to
/// deserialize and, via `serde_json::from_value` in `list_trajectories`,
/// collapse the ENTIRE listing to `ERR_API_FAILED` -- one odd conversation
/// hiding every other one. `Option<T>` fields don't need the same
/// attribute (serde already treats a missing field as `None` for them),
/// but it is kept here anyway for symmetry with `trajectory_id` below.
#[derive(Deserialize)]
struct RawSummary {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, rename = "stepCount")]
    step_count: Option<u32>,
    #[serde(default, rename = "trajectoryId")]
    trajectory_id: String,
    #[serde(default)]
    workspaces: Vec<RawWorkspace>,
}

#[derive(Deserialize)]
struct RawWorkspace {
    #[serde(default, rename = "workspaceFolderAbsoluteUri")]
    workspace_folder_absolute_uri: Option<String>,
    #[serde(default, rename = "gitRootAbsoluteUri")]
    git_root_absolute_uri: Option<String>,
    #[serde(default, rename = "branchName")]
    branch_name: Option<String>,
}

/// Builds one `TrajectoryDescription` from a `(cascade id, summary)` entry
/// of `trajectorySummaries`, taking workspace/root/branch from the first
/// workspace entry -- the only one a single-workspace conversation ever has.
fn description_from(cascade_id: String, raw: RawSummary) -> TrajectoryDescription {
    let first_workspace = raw.workspaces.into_iter().next();
    let (workspace_uri, git_root, git_branch) = match first_workspace {
        Some(w) => (
            w.workspace_folder_absolute_uri,
            w.git_root_absolute_uri,
            w.branch_name,
        ),
        None => (None, None, None),
    };
    TrajectoryDescription {
        cascade_id,
        trajectory_id: raw.trajectory_id,
        workspace_uri,
        git_root,
        git_branch,
        summary: raw.summary,
        step_count: raw.step_count,
    }
}

fn descriptions_from(raw: RawListing) -> Vec<TrajectoryDescription> {
    raw.trajectory_summaries
        .into_iter()
        .map(|(cascade_id, summary)| description_from(cascade_id, summary))
        .collect()
}

/// The live implementation: POSTs to the discovered local API.
pub(crate) struct HttpApi {
    endpoint: Endpoint,
    client: reqwest::Client,
}

impl HttpApi {
    pub(crate) fn new(endpoint: Endpoint) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| anyhow::anyhow!(ERR_API_FAILED))?;
        Ok(Self { endpoint, client })
    }

    /// POSTs `body` to `method` and returns the parsed JSON response.
    /// Every failure mode collapses to `ERR_API_FAILED` -- see the module
    /// doc comment for why the underlying `reqwest`/`serde_json` errors are
    /// discarded rather than propagated.
    async fn call(&self, method: &str, body: serde_json::Value) -> Result<serde_json::Value> {
        let url = format!(
            "http://127.0.0.1:{}{SERVICE_PATH}/{method}",
            self.endpoint.port
        );
        let response = self
            .client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(CSRF_HEADER, &self.endpoint.token)
            .json(&body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!(ERR_API_FAILED))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(ERR_API_FAILED));
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(|_| anyhow::anyhow!(ERR_API_FAILED))
    }
}

impl AntigravityApi for HttpApi {
    async fn list_trajectories(&self) -> Result<Vec<TrajectoryDescription>> {
        let body = self.call(LIST_METHOD, serde_json::json!({})).await?;
        let raw: RawListing =
            serde_json::from_value(body).map_err(|_| anyhow::anyhow!(ERR_API_FAILED))?;
        Ok(descriptions_from(raw))
    }

    async fn fetch_steps(&self, cascade_id: &str) -> Result<serde_json::Value> {
        self.call(STEPS_METHOD, serde_json::json!({"cascadeId": cascade_id}))
            .await
    }
}

/// Path to a committed fixture under `tests/fixtures/antigravity/`.
#[cfg(test)]
pub(crate) fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antigravity")
        .join(name)
}

/// The cascade id `listing.json`'s one entry is keyed on -- also the
/// cascade id both step fixtures describe (see the fixture README).
#[cfg(test)]
const FIXTURE_CASCADE_ID: &str = "39f32a85-508b-430a-98fb-a67e89b4e689";

/// A `fetch_steps` call for an id this test double does not recognize.
/// Falling through to a plausible-looking fixture instead of erroring would
/// reproduce, inside the double, exactly the non-self-diagnosing trap the
/// module doc comment warns about for the real API: a wrong id must look
/// like a wrong id, not like a different, valid conversation.
#[cfg(test)]
pub(crate) const ERR_FIXTURE_UNKNOWN_CASCADE_ID: &str = "antigravity-fixture-unknown-cascade-id";

/// Serves the committed fixtures instead of a live IDE. `fetch_steps` keys
/// on the string `"multi-turn"` and on [`FIXTURE_CASCADE_ID`] itself (the
/// id a real list-then-fetch round trip resolves to) to serve
/// `steps-multi-turn.json` -- `stepCount: 48` in `listing.json` matches
/// that capture's post-second-turn state -- and on `"single-turn"` to serve
/// `steps-single-turn.json`. Any other id is refused with
/// `ERR_FIXTURE_UNKNOWN_CASCADE_ID` rather than silently answering with one
/// of the two fixtures.
#[cfg(test)]
pub(crate) struct FixtureApi;

#[cfg(test)]
impl FixtureApi {
    pub(crate) fn new() -> Self {
        FixtureApi
    }
}

#[cfg(test)]
impl AntigravityApi for FixtureApi {
    async fn list_trajectories(&self) -> Result<Vec<TrajectoryDescription>> {
        let text = std::fs::read_to_string(fixture_path("listing.json"))?;
        let raw: RawListing = serde_json::from_str(&text)?;
        Ok(descriptions_from(raw))
    }

    async fn fetch_steps(&self, cascade_id: &str) -> Result<serde_json::Value> {
        let name = if cascade_id == "multi-turn" || cascade_id == FIXTURE_CASCADE_ID {
            "steps-multi-turn.json"
        } else if cascade_id == "single-turn" {
            "steps-single-turn.json"
        } else {
            anyhow::bail!(ERR_FIXTURE_UNKNOWN_CASCADE_ID);
        };
        let text = std::fs::read_to_string(fixture_path(name))?;
        Ok(serde_json::from_str(&text)?)
    }
}

/// The description matching the conversation in the step fixtures, so
/// `convert` tests do not each rebuild one. Derived from `listing.json`'s
/// own entry for [`FIXTURE_CASCADE_ID`] -- not hand-copied -- so a
/// recapture of that fixture can never leave this silently out of sync
/// with what it actually contains.
#[cfg(test)]
pub(crate) fn desc_fixture() -> TrajectoryDescription {
    let text = std::fs::read_to_string(fixture_path("listing.json"))
        .expect("listing fixture must be readable");
    let mut raw: RawListing =
        serde_json::from_str(&text).expect("listing fixture must parse as RawListing");
    let summary = raw
        .trajectory_summaries
        .remove(FIXTURE_CASCADE_ID)
        .expect("listing fixture must carry the known cascade id");
    description_from(FIXTURE_CASCADE_ID.to_string(), summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn descriptions_parse_from_the_recorded_response() {
        let api = FixtureApi::new();
        let list = api.list_trajectories().await.expect("fixture must parse");
        assert!(!list.is_empty());
        let first = &list[0];
        assert!(
            first
                .workspace_uri
                .as_deref()
                .unwrap()
                .starts_with("file:///")
        );
        assert!(!first.trajectory_id.is_empty());
        assert!(!first.cascade_id.is_empty());
    }

    #[tokio::test]
    async fn steps_parse_from_the_recorded_multi_turn_response() {
        let api = FixtureApi::new();
        let doc = api
            .fetch_steps("multi-turn")
            .await
            .expect("fixture must parse");
        let steps = doc["steps"].as_array().expect("steps is an array");
        assert_eq!(steps.len(), 48);
    }

    #[tokio::test]
    async fn steps_parse_from_the_recorded_single_turn_response() {
        let api = FixtureApi::new();
        let doc = api
            .fetch_steps("single-turn")
            .await
            .expect("fixture must parse");
        let steps = doc["steps"].as_array().expect("steps is an array");
        assert_eq!(steps.len(), 23);
    }

    #[test]
    fn desc_fixture_matches_the_step_fixtures_conversation() {
        let desc = desc_fixture();
        assert_eq!(desc.trajectory_id, "f1422752-2ec0-45ad-a5cc-0068a6b2ffd7");
        assert_eq!(desc.cascade_id, "39f32a85-508b-430a-98fb-a67e89b4e689");
        assert_eq!(
            desc.workspace_uri.as_deref(),
            Some("file:///Users/anonymized/code/trace-commons-server")
        );
        assert_eq!(
            desc.git_root.as_deref(),
            Some("file:///Users/anonymized/code/trace-commons-server")
        );
        assert_eq!(
            desc.git_branch.as_deref(),
            Some("settlement-disabled-honest-receipt")
        );
        assert_eq!(desc.summary.as_deref(), Some("Repository Overview Request"));
        assert_eq!(desc.step_count, Some(48));
    }

    /// Proto3 JSON omits any field left at its default value, so a listing
    /// entry with no `trajectoryId` at all (an empty string, in proto3
    /// terms) must still parse -- and must not take the rest of the
    /// listing down with it.
    #[test]
    fn a_listing_entry_missing_trajectory_id_still_parses() {
        let raw: RawListing = serde_json::from_value(serde_json::json!({
            "trajectorySummaries": {
                "cascade-only": {}
            }
        }))
        .expect("a listing entry with no trajectoryId must still parse");
        let list = descriptions_from(raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].cascade_id, "cascade-only");
        assert_eq!(list[0].trajectory_id, "");
    }

    #[tokio::test]
    async fn fetch_steps_refuses_an_unrecognized_cascade_id() {
        let api = FixtureApi::new();
        let err = api
            .fetch_steps("not-a-fixture-cascade-id")
            .await
            .expect_err("an unknown cascade id must not silently resolve to a fixture");
        assert_eq!(err.to_string(), ERR_FIXTURE_UNKNOWN_CASCADE_ID);
    }

    /// The round trip that was impossible before this fix: resolve a
    /// conversation through the listing, take its cascade id, and fetch
    /// that cascade's steps -- exactly the real list-then-fetch flow.
    #[tokio::test]
    async fn a_listed_trajectorys_cascade_id_fetches_its_own_steps() {
        let api = FixtureApi::new();
        let list = api.list_trajectories().await.expect("fixture must parse");
        let entry = list
            .iter()
            .find(|d| d.cascade_id == FIXTURE_CASCADE_ID)
            .expect("listing fixture carries the known cascade id");

        let doc = api
            .fetch_steps(&entry.cascade_id)
            .await
            .expect("fixture must parse");
        let steps = doc["steps"].as_array().expect("steps is an array");
        assert_eq!(steps.len(), 48);
    }
}
