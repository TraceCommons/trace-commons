//! The `import-antigravity` command: list the running IDE's conversations,
//! keep the ones belonging to this project, convert them to Trajectory-v1,
//! and stage them where the existing `trajectory` source will find them.
//!
//! Staging goes to the `trajectories` folder inside the contributor state
//! directory -- the same `Scope::Staging` the `trajectory` source already
//! reads. Never the contributor's project: a trace file dropped into a repo
//! is a file somebody eventually commits.
//!
//! **An empty import is never silent.** Filtering happens at listing time,
//! on `workspaceUri`, before anything is fetched, and every conversation
//! turned away is counted. Without that count `import --project .` in a
//! directory Antigravity has never seen produces exactly the output it
//! produces when the IDE has no conversations at all, and the contributor
//! has no way to tell a mis-scoped run from an empty one. That
//! indistinguishability is the defect this command exists to avoid, so the
//! counts are part of the contract, not decoration.
//!
//! **A partial import is not silent either.** Conversations are staged as
//! the loop runs, and the staging directory is auto-discovered by the
//! trajectory source, so a run that dies halfway has still collected
//! files that a later bare `submit` will offer. Reporting only the error
//! would leave the contributor believing nothing happened, which is worse
//! than the empty-run case: they would be offered work they were never
//! told was collected. Every run therefore yields an [`ImportOutcome`]
//! carrying both what was staged and the failure that stopped it, and the
//! command prints the counts before surfacing the error.
//!
//! Filtering is on `workspaceUri` and fetching is on `cascadeId` -- see
//! [`super::client`] for why those two identifiers must not be confused.
//!
//! The endpoint's port and CSRF token are live credentials. Nothing here
//! formats an [`Endpoint`], and every API failure arrives already collapsed
//! to a fixed label by `client`/`endpoint`, so neither can reach an error,
//! a log line, or the `--json` output.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use super::client::{AntigravityApi, HttpApi, TrajectoryDescription};
use super::convert::{ERR_NO_CONTENT, cwd_from_workspace_uri, to_trajectory_v1};
use super::endpoint::discover;
use crate::config::ConfigStore;
use crate::source::TRAJECTORY_STAGING_SUBDIR;

/// The working directory could not be read, so there is no project to scope
/// the import to. Refused rather than falling back to "every conversation":
/// `--all` is how a contributor asks for that, and it must stay a thing
/// they said.
pub(crate) const ERR_NO_PROJECT: &str = "antigravity-no-project";

/// A cascade id that would not be a single safe path component. Ids are
/// uuids in every capture, but the id becomes a FILE NAME here, and it
/// arrives from a process this command does not own; a `..` or a separator
/// in one must not be able to steer a write out of the staging directory.
pub(crate) const ERR_UNSAFE_CASCADE_ID: &str = "antigravity-unsafe-cascade-id";

/// What one import run did, in the terms a contributor needs to tell an
/// empty run apart from a mis-scoped one.
pub struct ImportSummary {
    /// Conversations converted and written to `staged_dir`.
    pub imported: usize,
    /// Conversations the running instance exposed that belong to some other
    /// project -- including ones whose workspace it did not report at all,
    /// which cannot be shown to belong to this one.
    pub skipped_other_projects: usize,
    /// Conversations in scope that converted to no transcript at all: no
    /// user turn, no assistant turn. Counted separately so "matched but
    /// empty" never reads as "matched nothing".
    pub skipped_no_content: usize,
    /// Where the staged files went.
    pub staged_dir: PathBuf,
}

/// What one import run did, and the failure that stopped it if there was
/// one. The two travel together because a failed run has usually still
/// staged files, and those files are live: the trajectory source
/// auto-discovers the staging directory, so a later bare `submit` offers
/// them. Reporting the error alone would tell the contributor nothing
/// happened while something did.
pub struct ImportOutcome {
    pub summary: ImportSummary,
    /// The failure that ended the run early, if any. The command still
    /// exits non-zero on this; it prints the summary first.
    pub error: Option<anyhow::Error>,
}

impl ImportOutcome {
    /// The summary when the run finished, the failure when it did not.
    /// For callers that only care whether the whole run succeeded.
    pub fn into_result(self) -> Result<ImportSummary> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.summary),
        }
    }
}

/// Whether a cascade id is safe to use as a file name: one path component,
/// no separators, no `.`/`..`, nothing but the uuid alphabet.
fn is_safe_cascade_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The single form of a path both sides of the comparison are reduced to:
/// the kernel's answer for the longest prefix that exists, with whatever is
/// left below it appended as written.
///
/// **The kernel is asked about the path AS WRITTEN, never about a lexically
/// rearranged version of it.** Collapsing `..` before resolving is wrong and
/// dangerous: `--project /mine/link/..` where `link -> /theirs/target` means
/// `/theirs` to the kernel, but collapsing `link/..` lexically yields
/// `/mine`, and canonicalizing `/mine` only confirms `/mine`. The filter
/// then covers every repo under `/mine`, and unrelated conversations get
/// staged into a contribution. Over-matching is the worse direction here: a
/// missed match costs the contributor one import, a false match publishes
/// work that was never theirs to submit.
///
/// Walking up from the full path -- rather than giving up when it does not
/// exist -- is what keeps that guarantee for a path that is only partly
/// real, which is a case that genuinely occurs: a conversation recorded on a
/// checkout this machine no longer has, or a `--project` naming a directory
/// since deleted. Every component the kernel can resolve, including a `..`
/// through a symlink, is resolved by it. The tail below cannot be hiding a
/// symlink, because it does not exist.
///
/// `Components` drops `.` on its own -- `/a/b/c`.starts_with(`/a/b/.`) is
/// already TRUE -- so nothing here has to handle `CurDir`, and the tail this
/// re-appends carries none.
fn resolved_form(path: &Path) -> PathBuf {
    let components: Vec<std::path::Component<'_>> = path.components().collect();
    for split in (1..=components.len()).rev() {
        let prefix: PathBuf = components[..split].iter().collect();
        if let Ok(resolved) = std::fs::canonicalize(&prefix) {
            let mut out = resolved;
            out.extend(components[split..].iter().map(|c| c.as_os_str()));
            return out;
        }
    }
    // Defensive: an absolute path always reaches a root that canonicalizes,
    // and `resolve_project` only ever produces absolute paths, so this is
    // unreachable in practice rather than a second comparison strategy.
    components.iter().collect()
}

/// Resolve the `--project` argument into an absolute filter path.
///
/// A relative argument is joined onto the working directory rather than
/// compared as written. `--project .` is the obvious thing to type given the
/// flag's own help text, and comparing it as written makes every workspace
/// path fail `starts_with`, so every conversation would be reported as
/// belonging to another project.
/// A `--project` path that is not there is refused rather than filtered
/// with. `submit --project` has always said so -- `discover_filtered`
/// rejects a missing path and its comment gives the reason: silent-empty
/// makes a typo indistinguishable from "this project has no traces". The
/// same argument applies with more force here, because this command's whole
/// premise is that an empty import must never be ambiguous.
///
/// Only the contributor's own argument is held to this. The other side of
/// the comparison -- the workspace a conversation was recorded in -- is
/// still reduced by `resolved_form`, which deliberately tolerates a path
/// this machine no longer has: a conversation from a deleted checkout is a
/// real thing to hold, while a `--project` naming a directory that is not
/// there is a mistake to report.
fn resolve_project(project: Option<&str>) -> Result<PathBuf> {
    let raw = match project {
        Some(p) => PathBuf::from(p),
        None => return std::env::current_dir().map_err(|_| anyhow!(ERR_NO_PROJECT)),
    };
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|_| anyhow!(ERR_NO_PROJECT))?
            .join(raw)
    };
    if !absolute.exists() {
        return Err(anyhow!(
            "--project path {} does not exist",
            absolute.display()
        ));
    }
    Ok(absolute)
}

/// Whether a listed conversation belongs to `project`.
///
/// A conversation whose workspace the API did not report does NOT match: it
/// cannot be shown to belong here, and staging it would attribute someone
/// else's project to this one. `--all` is the way to ask for those.
fn matches_project(desc: &TrajectoryDescription, project: &Path) -> bool {
    let Some(uri) = desc.workspace_uri.as_deref() else {
        return false;
    };
    let Some(cwd) = cwd_from_workspace_uri(uri) else {
        return false;
    };
    resolved_form(Path::new(&cwd)).starts_with(resolved_form(project))
}

/// Write one converted conversation into the staging directory at 0600.
fn stage(staging: &Path, cascade_id: &str, records: &[serde_json::Value]) -> Result<PathBuf> {
    if !is_safe_cascade_id(cascade_id) {
        return Err(anyhow!(ERR_UNSAFE_CASCADE_ID));
    }
    let path = staging.join(format!("{cascade_id}.json"));
    let body = serde_json::to_vec_pretty(&records)?;
    // The mode is set AT CREATION, not after the write: creating the file
    // at the umask's mode and narrowing it afterwards leaves a window in
    // which a whole conversation is world-readable on disk.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    std::io::Write::write_all(&mut file, &body)?;
    Ok(path)
}

/// The testable core: everything but the live API and the real staging path.
///
/// Generic over the API rather than taking `&dyn AntigravityApi`, because
/// that trait's methods are `async fn` and so it is not dyn-compatible.
pub(crate) async fn import_with<A: AntigravityApi>(
    api: &A,
    staging: &Path,
    filter: Option<&Path>,
) -> ImportOutcome {
    let mut summary = ImportSummary {
        imported: 0,
        skipped_other_projects: 0,
        skipped_no_content: 0,
        staged_dir: staging.to_path_buf(),
    };
    macro_rules! stop {
        ($e:expr) => {
            return ImportOutcome {
                summary,
                error: Some($e),
            }
        };
    }

    let listing = match api.list_trajectories().await {
        Ok(listing) => listing,
        Err(e) => stop!(e),
    };

    // Filter first, fetch second: a conversation belonging to another
    // project is never fetched at all, so this command reads no more of the
    // operator's history than the run asked for.
    let selected: Vec<TrajectoryDescription> = listing
        .into_iter()
        .filter(|desc| match &filter {
            None => true,
            Some(project) => {
                let keep = matches_project(desc, project);
                if !keep {
                    summary.skipped_other_projects += 1;
                }
                keep
            }
        })
        .collect();

    if !selected.is_empty() {
        if let Err(e) = create_staging_dir(staging) {
            stop!(e);
        }
    }

    for desc in &selected {
        // By CASCADE id, never `trajectory_id`: the wrong one returns the
        // same generic "not found" an empty request does.
        let steps = match api.fetch_steps(&desc.cascade_id).await {
            Ok(steps) => steps,
            Err(e) => stop!(e),
        };
        // A conversation with no transcript is skipped, not fatal: one
        // empty conversation in the listing must not take the rest of the
        // import down with it. It is still counted.
        //
        // Matched on the LABEL, not on "any error". `ERR_NO_CONTENT` is the
        // only failure `to_trajectory_v1` returns today, but a conversion
        // failure added later is not an empty conversation and must not be
        // reported to the contributor as one -- it fails the run instead.
        let records = match to_trajectory_v1(&steps, desc) {
            Ok(records) => records,
            Err(e) if e.to_string() == ERR_NO_CONTENT => {
                summary.skipped_no_content += 1;
                continue;
            }
            Err(e) => stop!(e),
        };
        if let Err(e) = stage(staging, &desc.cascade_id, &records) {
            stop!(e);
        }
        summary.imported += 1;
    }

    ImportOutcome {
        summary,
        error: None,
    }
}

/// What to actually do about a discovery failure, or `None` for a failure
/// that already reads as a sentence.
///
/// The two labels below are the ones a contributor meets before anything
/// else works, and both were reported as bare slugs: `Error:
/// antigravity-not-running` told someone neither that the IDE has to be
/// running nor that the project has to be open in it, which are the two
/// things this command needs and cannot arrange for itself.
///
/// The equivalent confusion on the other side -- running the import from
/// the wrong directory -- has had a real sentence since the command shipped
/// (`zero_import_explanation` in `commands`). This is the same courtesy for
/// the failure that comes first.
///
/// **This does not weaken the content-free error rule.** That rule exists
/// because the endpoint holds a live CSRF token and a port, so its errors
/// are fixed `&'static str` labels that cannot carry either. Mapping a
/// known fixed label to fixed prose at the boundary adds no runtime value
/// to the message: nothing here interpolates anything.
pub(crate) fn discovery_guidance(error: &anyhow::Error) -> Option<&'static str> {
    match error.to_string().as_str() {
        super::endpoint::ERR_NOT_RUNNING => Some(
            "Antigravity does not appear to be running. Its conversations are only \
             readable through the local API the IDE serves, so start Antigravity, \
             open the project you want to import, and run this again.",
        ),
        super::endpoint::ERR_API_NOT_FOUND => Some(
            "Antigravity is running, but its local API did not answer on any port \
             this looked at. If the IDE has just started, give it a moment and try \
             again; if it stays unreachable, the running build may not serve the \
             API this command needs.",
        ),
        _ => None,
    }
}

/// Create the staging directory at 0700.
fn create_staging_dir(staging: &Path) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staging, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The live command: discover the running IDE, then run the import against
/// the state directory's trajectory staging folder.
pub async fn import_antigravity(
    store: &ConfigStore,
    project: Option<&str>,
    all: bool,
) -> Result<ImportOutcome> {
    // Resolved before the IDE is even looked for, so a run that cannot know
    // its own project -- or was handed one that is not there -- fails before
    // it has read anything at all.
    //
    // `None` here means "no filter", which is exactly `--all`. Resolving at
    // the entry rather than inside `import_with` is what removes the fourth
    // state the old (project, all) pair could express: not-all with no
    // project, which had to be silently rewritten to the working directory
    // deep inside the run.
    let filter = if all {
        None
    } else {
        Some(resolve_project(project)?)
    };
    let endpoint = discover().await?;
    let api = HttpApi::new(endpoint)?;
    let staging = store.dir().join(TRAJECTORY_STAGING_SUBDIR);
    Ok(import_with(&api, &staging, filter.as_deref()).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::antigravity::client::{FixtureApi, desc_fixture};

    /// The failure a contributor is most likely to hit first says what to
    /// do about it.
    ///
    /// `import-antigravity` reported `Error: antigravity-not-running` and
    /// stopped -- a label, with no hint that the IDE has to be started, or
    /// that the project has to be open in it before its conversations are
    /// readable. The neighbouring confusion, running the import from the
    /// wrong directory, has had a real sentence since the command shipped
    /// (`zero_import_explanation`); this one had nothing, and it is the
    /// one a first attempt is most likely to produce.
    #[test]
    fn the_two_discovery_failures_say_what_to_do_about_them() {
        for label in [
            crate::antigravity::endpoint::ERR_NOT_RUNNING,
            crate::antigravity::endpoint::ERR_API_NOT_FOUND,
        ] {
            let guidance = discovery_guidance(&anyhow!(label))
                .unwrap_or_else(|| panic!("{label} must carry guidance"));
            assert!(
                guidance.contains("Antigravity"),
                "{label}: guidance must name the application: {guidance}"
            );
            assert!(
                guidance.len() > label.len(),
                "{label}: guidance must say more than the label did"
            );
        }
    }

    /// Only those two. Every other failure already reads as a sentence --
    /// the `--project` check added later says what is wrong with the path
    /// it was given -- and wrapping one in a second explanation would bury
    /// it.
    #[test]
    fn other_failures_are_left_to_speak_for_themselves() {
        assert!(discovery_guidance(&anyhow!(ERR_NO_PROJECT)).is_none());
        assert!(discovery_guidance(&anyhow!("--project path /nope does not exist")).is_none());
    }

    /// The project the step fixtures' conversation was recorded in.
    const FIXTURE_PROJECT: &str = "/Users/anonymized/code/trace-commons-server";
    /// The cascade id `listing.json` is keyed on, and the one both step
    /// fixtures describe.
    const FIXTURE_CASCADE: &str = "39f32a85-508b-430a-98fb-a67e89b4e689";
    /// A second cascade id `FixtureApi` knows how to serve steps for.
    const OTHER_CASCADE: &str = "single-turn";
    /// An id this module's double answers with an empty step list, so the
    /// "matched but empty" path has something to exercise.
    const EMPTY_CASCADE: &str = "empty-conversation";

    /// A listing this test controls, with step fetches delegated to
    /// [`FixtureApi`].
    ///
    /// `listing.json` holds exactly ONE conversation, so a partition test
    /// built on it is vacuous: `imported == 1` is satisfied whether or not
    /// the filter does anything at all. This double supplies a second
    /// conversation in a different workspace so the split can actually be
    /// asserted.
    struct ListingApi {
        /// `(cascade id, workspace uri)` for each listed conversation.
        entries: Vec<(String, Option<String>)>,
    }

    impl ListingApi {
        fn new(entries: &[(&str, Option<&str>)]) -> Self {
            Self {
                entries: entries
                    .iter()
                    .map(|(id, uri)| (id.to_string(), uri.map(str::to_string)))
                    .collect(),
            }
        }
    }

    impl AntigravityApi for ListingApi {
        async fn list_trajectories(&self) -> Result<Vec<TrajectoryDescription>> {
            Ok(self
                .entries
                .iter()
                .map(|(cascade_id, workspace_uri)| TrajectoryDescription {
                    cascade_id: cascade_id.clone(),
                    trajectory_id: format!("trajectory-of-{cascade_id}"),
                    workspace_uri: workspace_uri.clone(),
                    git_root: workspace_uri.clone(),
                    git_branch: None,
                    summary: None,
                    step_count: None,
                })
                .collect())
        }

        async fn fetch_steps(&self, cascade_id: &str) -> Result<serde_json::Value> {
            if cascade_id == EMPTY_CASCADE {
                return Ok(serde_json::json!({"steps": []}));
            }
            FixtureApi::new().fetch_steps(cascade_id).await
        }
    }

    /// The two-conversation listing the partition tests share: one in the
    /// fixture project, one in a sibling project under the same parent.
    fn two_projects() -> ListingApi {
        ListingApi::new(&[
            (
                FIXTURE_CASCADE,
                Some("file:///Users/anonymized/code/trace-commons-server"),
            ),
            (
                OTHER_CASCADE,
                Some("file:///Users/anonymized/code/some-other-repo"),
            ),
        ])
    }

    /// The names of the files staged in `dir`, sorted.
    fn staged_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn only_conversations_matching_the_project_are_staged() {
        let dir = tempfile::tempdir().unwrap();
        let api = two_projects();
        let summary = import_with(&api, dir.path(), Some(Path::new(FIXTURE_PROJECT)))
            .await
            .into_result()
            .expect("import must succeed");

        // The partition, not just the count: the other project's
        // conversation was listed, was turned away, and did not reach disk.
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped_other_projects, 1);
        assert_eq!(summary.skipped_no_content, 0);
        assert_eq!(
            staged_names(dir.path()),
            vec![format!("{FIXTURE_CASCADE}.json")],
            "one file per imported conversation, and only the matching one"
        );
    }

    #[tokio::test]
    async fn a_project_filter_matching_nothing_stages_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let api = two_projects();
        let summary = import_with(&api, dir.path(), Some(Path::new("/somewhere/else")))
            .await
            .into_result()
            .unwrap();
        assert_eq!(summary.imported, 0);
        assert_eq!(
            summary.skipped_other_projects, 2,
            "an empty result must be distinguishable from 'no conversations exist'"
        );
        assert!(staged_names(dir.path()).is_empty());
    }

    /// `--all` takes what the project filter turned away, and reports
    /// nothing skipped, because nothing was.
    #[tokio::test]
    async fn all_imports_every_conversation_and_skips_none() {
        let dir = tempfile::tempdir().unwrap();
        let api = two_projects();
        let summary = import_with(&api, dir.path(), None)
            .await
            .into_result()
            .unwrap();
        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped_other_projects, 0);
        assert_eq!(
            staged_names(dir.path()),
            vec![
                format!("{FIXTURE_CASCADE}.json"),
                format!("{OTHER_CASCADE}.json")
            ]
        );
    }

    /// A conversation that matches the project but converts to no
    /// transcript is counted on its own line. Without this the run reports
    /// `imported: 0, skipped_other_projects: 0` -- indistinguishable from
    /// an instance with no conversations at all.
    #[tokio::test]
    async fn a_matched_but_empty_conversation_is_counted_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let api = ListingApi::new(&[
            (
                FIXTURE_CASCADE,
                Some("file:///Users/anonymized/code/trace-commons-server"),
            ),
            (
                EMPTY_CASCADE,
                Some("file:///Users/anonymized/code/trace-commons-server"),
            ),
        ]);
        let summary = import_with(&api, dir.path(), Some(Path::new(FIXTURE_PROJECT)))
            .await
            .into_result()
            .expect("one empty conversation must not fail the run");
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped_no_content, 1);
        assert_eq!(summary.skipped_other_projects, 0);
        assert_eq!(
            staged_names(dir.path()),
            vec![format!("{FIXTURE_CASCADE}.json")]
        );
    }

    /// The staged file is named for the CASCADE id -- the identifier that
    /// fetched it -- not the different uuid recorded inside the
    /// conversation as `trajectoryId`.
    #[tokio::test]
    async fn the_staged_file_is_named_for_the_cascade_id() {
        let dir = tempfile::tempdir().unwrap();
        let api = FixtureApi::new();
        import_with(&api, dir.path(), Some(Path::new(FIXTURE_PROJECT)))
            .await
            .into_result()
            .unwrap();
        let listed = api.list_trajectories().await.unwrap();
        let desc = &listed[0];
        assert_ne!(desc.cascade_id, desc.trajectory_id);
        assert!(
            dir.path()
                .join(format!("{}.json", desc.cascade_id))
                .exists()
        );
        assert!(
            !dir.path()
                .join(format!("{}.json", desc.trajectory_id))
                .exists()
        );
    }

    /// The whole point of staging where it stages: the `trajectory` source
    /// must accept the file, and it fails closed on an orphaned
    /// `tool_call_id`, so a conversion that dropped a call would be
    /// rejected here rather than at submit time.
    ///
    /// Parsing is not enough on its own -- a file that parses can still
    /// have lost or reordered turns on the way to disk -- so the turns are
    /// asserted through the staged BYTES, by content and in order.
    /// The end a contributor actually sees: after importing, `list` names
    /// the conversation `antigravity`, not the adapter that stores it.
    ///
    /// The unit test on `session_row` pins the rendering; this pins the
    /// whole path -- convert writes `meta.source`, staging puts the file
    /// where the trajectory source reads it, and discovery carries that
    /// value out on the ref. A break anywhere along it lands here.
    #[tokio::test]
    async fn a_staged_conversation_is_discovered_as_antigravity_not_as_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let api = FixtureApi::new();
        import_with(&api, dir.path(), Some(Path::new(FIXTURE_PROJECT)))
            .await
            .into_result()
            .unwrap();

        // The staging directory is exactly the scope the trajectory source
        // auto-reads, so point one at it the way the daemon does.
        let source =
            crate::source::trajectory::TrajectorySource::auto(None, Some(dir.path().to_path_buf()));
        let refs = crate::source::TraceSource::discover(&source).unwrap();
        assert_eq!(refs.len(), 1, "the staged conversation must be discovered");

        assert_eq!(
            refs[0].source,
            crate::source::SOURCE_TRAJECTORY,
            "the adapter is still what loads it -- `source` must stay resolvable"
        );
        assert_eq!(
            refs[0].declared_source.as_deref(),
            Some("antigravity"),
            "but what it declares itself to be is what a contributor is shown"
        );
    }

    #[tokio::test]
    async fn a_staged_file_round_trips_its_turns_through_the_trajectory_source() {
        let dir = tempfile::tempdir().unwrap();
        let api = FixtureApi::new();
        import_with(&api, dir.path(), Some(Path::new(FIXTURE_PROJECT)))
            .await
            .into_result()
            .unwrap();
        let staged = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .next()
            .expect("one staged file");
        let bytes = std::fs::read(staged.path()).unwrap();
        let parsed = crate::source::trajectory::parse_trajectory(&bytes)
            .expect("the trajectory source must accept what this command stages");

        assert_eq!(parsed.source, "antigravity");
        assert_eq!(parsed.cwd.as_deref(), Some(FIXTURE_PROJECT));

        let user_turns: Vec<&str> = parsed
            .events
            .iter()
            .filter(|e| matches!(e.kind, crate::source::SessionEventKind::User))
            .filter_map(|e| e.content.as_deref())
            .collect();
        assert_eq!(
            user_turns,
            vec!["Tell me about this repo", "What should we more on next?"],
            "both turns of the multi-turn capture survive staging, in order"
        );
        // Every tool result the reader accepted resolved to a call it saw;
        // the reader fails closed on one that does not, so the presence of
        // results at all is what makes the parse above meaningful.
        assert!(
            parsed
                .events
                .iter()
                .any(|e| matches!(e.kind, crate::source::SessionEventKind::ToolResult))
        );
    }

    /// `--project .` is the obvious thing to type given the flag's own help
    /// text, and a relative path never `starts_with` an absolute one, so
    /// compared as typed it matches NOTHING and every conversation is
    /// reported as another project's. Joining onto the working directory is
    /// the whole fix -- the `.` itself is harmless, since `Path::components`
    /// drops it.
    #[test]
    fn a_relative_project_argument_resolves_to_a_filter_that_matches() {
        let cwd = std::env::current_dir().unwrap();
        let resolved = resolve_project(Some(".")).unwrap();
        assert!(resolved.is_absolute());

        let mut desc = desc_fixture();
        desc.workspace_uri = Some(format!("file://{}", cwd.display()));
        assert!(
            matches_project(&desc, &resolved),
            "a conversation recorded in the working directory must match `--project .`"
        );
        desc.workspace_uri = Some(format!("file://{}/src", cwd.display()));
        assert!(
            matches_project(&desc, &resolved),
            "and so must one recorded in a subdirectory of it"
        );
        // The raw comparison the old code performed, shown to fail: this
        // is why the argument is resolved rather than compared as typed.
        assert!(!Path::new(&format!("{}/src", cwd.display())).starts_with("."));
    }

    /// A `--project` path that is not there is a typo, not an empty result.
    ///
    /// `submit --project` has always said so -- `discover_filtered` refuses
    /// a missing path with "does it exist?" and its comment gives the
    /// reason: silent-empty makes a typo indistinguishable from "this
    /// project has no traces". `import --project` accepted anything and
    /// filtered with it, so one mistyped character reported every
    /// conversation as belonging to another project, which is the same
    /// output as a correct run against a project with nothing in it.
    #[test]
    fn a_project_path_that_does_not_exist_is_refused_rather_than_matching_nothing() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("no-such-checkout");
        assert!(!missing.exists());

        let err = resolve_project(Some(missing.to_str().unwrap()))
            .expect_err("a missing --project path must not resolve");
        let message = err.to_string();
        assert!(
            message.contains("no-such-checkout"),
            "the error must name the path the contributor typed: {message}"
        );

        // A relative path is resolved before the check, so a missing one is
        // refused for the same reason rather than slipping through as
        // "cannot canonicalize, compare it as typed".
        assert!(resolve_project(Some("no-such-checkout-either")).is_err());

        // And a path that IS there still resolves, so the check rejects
        // only what it is meant to.
        assert!(resolve_project(Some(root.path().to_str().unwrap())).is_ok());
    }

    /// The premise a lexical normalization step was built on, checked
    /// rather than assumed: `Path::components` drops `.` itself, so a `.`
    /// component never broke a comparison and nothing needs to strip one.
    /// A leading `.` IS kept, which is why a bare relative `--project .`
    /// still had to be joined onto the working directory.
    #[test]
    fn a_dot_component_is_already_transparent_to_path_comparison() {
        assert!(Path::new("/a/b/c").starts_with("/a/b/."));
        assert!(Path::new("/a/b/c").starts_with("/a/./b"));
        assert!(!Path::new("/a/b/c").starts_with("."));
    }

    /// macOS reports `/private/tmp` where a contributor types `/tmp`. The
    /// IDE gives the resolved path and the contributor gives the link, and
    /// a purely lexical comparison would call that a different project --
    /// silently dropping the contributor's own conversations.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_project_path_matches_the_resolved_workspace() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real-project");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("linked-project");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let mut desc = desc_fixture();
        // The API reports the fully resolved path.
        let resolved = std::fs::canonicalize(&real).unwrap();
        desc.workspace_uri = Some(format!("file://{}", resolved.display()));

        assert!(
            matches_project(&desc, &link),
            "the contributor's symlinked path must match the resolved one the IDE reports"
        );
    }

    /// A `..` that traverses a symlink means, to the kernel, whatever the
    /// link points at -- not the lexical parent. Collapsing `link/..`
    /// lexically before resolving turns `--project /mine/link/..` into a
    /// filter over `/mine`, which stages every OTHER project under `/mine`
    /// into the contribution. Over-matching is the direction that publishes
    /// work that was never the contributor's to submit.
    #[cfg(unix)]
    #[test]
    fn a_dotdot_through_a_symlink_does_not_widen_the_filter() {
        let root = tempfile::tempdir().unwrap();
        let mine = root.path().join("mine");
        let theirs = root.path().join("theirs");
        std::fs::create_dir_all(mine.join("someone-elses-repo")).unwrap();
        std::fs::create_dir_all(theirs.join("target")).unwrap();
        // `mine/link` -> `theirs/target`, so `mine/link/..` IS `theirs`.
        std::os::unix::fs::symlink(theirs.join("target"), mine.join("link")).unwrap();

        let project = mine.join("link").join("..");
        assert_eq!(
            resolved_form(&project),
            std::fs::canonicalize(&theirs).unwrap(),
            "the kernel's answer for the path as written, not the lexical parent"
        );

        let mut desc = desc_fixture();
        desc.workspace_uri = Some(format!(
            "file://{}",
            mine.join("someone-elses-repo").display()
        ));
        assert!(
            !matches_project(&desc, &project),
            "another project under /mine must not be swept in by a `..` through a symlink"
        );

        // And the conversation that really is under the resolved target
        // still matches, so the fix did not simply refuse everything.
        desc.workspace_uri = Some(format!("file://{}", theirs.join("target").display()));
        assert!(matches_project(&desc, &project));
    }

    /// A path that only partly exists: a real directory with a checkout
    /// under it that this machine does not have. `canonicalize` refuses the
    /// whole path, so a "canonicalize or give up" rule would compare the two
    /// sides in different forms -- here the project as the contributor typed
    /// it and the workspace as the IDE resolved it -- and silently drop the
    /// contributor's own conversation. Walking up to the deepest ancestor
    /// that DOES exist is what makes both sides comparable.
    ///
    /// The tempdir supplies the discrepancy for free on macOS, where it
    /// lives under `/var` and resolves to `/private/var`; the assertion
    /// below skips rather than passes vacuously if that is not the case.
    #[test]
    fn a_partly_nonexistent_project_still_matches_its_own_workspace() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(root.path()).unwrap();
        let gone = root.path().join("checkout-this-machine-does-not-have");
        assert!(!gone.exists());

        let project = gone.join("project");
        let mut desc = desc_fixture();
        // The IDE reports the resolved form; the contributor typed the
        // unresolved one. Below the tempdir neither can be canonicalized.
        desc.workspace_uri = Some(format!(
            "file://{}/checkout-this-machine-does-not-have/project/repo",
            canonical_root.display()
        ));
        assert!(
            matches_project(&desc, &project),
            "a project path that cannot be canonicalized whole must still match its own subtree"
        );

        // It must not match by way of the existing prefix the two sides
        // share: the tempdir resolves, the rest does not, and the rest is
        // what differs.
        desc.workspace_uri = Some(format!(
            "file://{}/checkout-this-machine-does-not-have/other/repo",
            canonical_root.display()
        ));
        assert!(!matches_project(&desc, &project));
        // Nor may a sibling whose name merely shares a prefix.
        desc.workspace_uri = Some(format!(
            "file://{}/checkout-this-machine-does-not-have/project-old/repo",
            canonical_root.display()
        ));
        assert!(!matches_project(&desc, &project));
    }

    /// A conversation with no workspace is not this project's by default.
    /// Staging it would attribute an unknown project's work to whatever
    /// directory the contributor happened to be standing in.
    #[test]
    fn a_conversation_with_no_workspace_never_matches_a_project() {
        let mut desc = desc_fixture();
        desc.workspace_uri = None;
        assert!(!matches_project(&desc, Path::new(FIXTURE_PROJECT)));
    }

    /// The filter is a subtree match, so standing in a parent directory
    /// covers the repos under it -- the same rule `submit --project` uses.
    #[test]
    fn a_parent_directory_matches_a_conversation_beneath_it() {
        let desc = desc_fixture();
        assert!(matches_project(&desc, Path::new("/Users/anonymized/code")));
        assert!(!matches_project(&desc, Path::new("/Users/anonymized/docs")));
        // A sibling whose name merely shares a prefix is not a parent.
        assert!(!matches_project(
            &desc,
            Path::new("/Users/anonymized/code/trace-commons-server-old")
        ));
    }

    /// An API that serves the first conversation and then fails, standing
    /// in for the IDE quitting mid-run.
    struct FailsAfterFirstApi {
        inner: ListingApi,
        served: std::cell::Cell<usize>,
    }

    impl AntigravityApi for FailsAfterFirstApi {
        async fn list_trajectories(&self) -> Result<Vec<TrajectoryDescription>> {
            self.inner.list_trajectories().await
        }

        async fn fetch_steps(&self, cascade_id: &str) -> Result<serde_json::Value> {
            if self.served.get() > 0 {
                return Err(anyhow!(crate::antigravity::client::ERR_API_FAILED));
            }
            self.served.set(self.served.get() + 1);
            self.inner.fetch_steps(cascade_id).await
        }
    }

    /// A run that dies partway has still staged files, and the staging
    /// directory is auto-discovered: a contributor told only
    /// `antigravity-api-failed` would conclude nothing happened, then be
    /// offered those conversations by a later bare `submit`. Both halves
    /// must survive -- the count of what reached disk, and the failure.
    #[tokio::test]
    async fn a_run_that_fails_partway_reports_what_it_already_staged() {
        let dir = tempfile::tempdir().unwrap();
        let api = FailsAfterFirstApi {
            inner: ListingApi::new(&[
                (
                    FIXTURE_CASCADE,
                    Some("file:///Users/anonymized/code/trace-commons-server"),
                ),
                (
                    OTHER_CASCADE,
                    Some("file:///Users/anonymized/code/trace-commons-server"),
                ),
            ]),
            served: std::cell::Cell::new(0),
        };
        let outcome = import_with(&api, dir.path(), Some(Path::new(FIXTURE_PROJECT))).await;

        let err = outcome.error.as_ref().expect("the failure must survive");
        assert_eq!(err.to_string(), crate::antigravity::client::ERR_API_FAILED);
        assert_eq!(
            outcome.summary.imported, 1,
            "the conversation staged before the failure must be reported"
        );
        assert_eq!(
            staged_names(dir.path()),
            vec![format!("{FIXTURE_CASCADE}.json")],
            "and it really is on disk, where `submit` will find it"
        );
    }

    #[test]
    fn a_cascade_id_that_is_not_one_safe_path_component_is_refused() {
        assert!(is_safe_cascade_id("39f32a85-508b-430a-98fb-a67e89b4e689"));
        assert!(!is_safe_cascade_id(""));
        assert!(!is_safe_cascade_id(".."));
        assert!(!is_safe_cascade_id("../../etc/passwd"));
        assert!(!is_safe_cascade_id("a/b"));

        let dir = tempfile::tempdir().unwrap();
        let err = stage(dir.path(), "../escape", &[]).expect_err("must refuse");
        assert_eq!(err.to_string(), ERR_UNSAFE_CASCADE_ID);
    }
}
