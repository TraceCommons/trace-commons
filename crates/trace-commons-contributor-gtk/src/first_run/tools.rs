//! Tools' answers, as the Mac keeps them (`FirstRunState.toolAnswers`,
//! `addedFolders`, `notFound`, `sessionRoots`; `ToolsScreenLayout`). Pure:
//! the GTK screen draws these and writes back through them, and the
//! planner's `Commit::Roots` sends what [`Tools::session_roots`] declares.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use trace_commons_contributor::source::{
    SOURCE_CLAUDE_CODE, SOURCE_CLINE, SOURCE_CODEX, SOURCE_GEMINI_CLI, SOURCE_OPENCODE,
    SOURCE_TRAJECTORY,
};

/// The tools a row can be offered for, in the Mac's ring order.
pub const TOOL_ORDER: [&str; 5] = [
    SOURCE_CLAUDE_CODE,
    SOURCE_CODEX,
    SOURCE_OPENCODE,
    SOURCE_GEMINI_CLI,
    SOURCE_CLINE,
];

/// One tool row's answer. `Undecided` is not an answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ToolAnswer {
    #[default]
    Undecided,
    Off,
    Watch(String),
}

/// What a folder picked with "add your tool" is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderKind {
    /// A session store of one of [`TOOL_ORDER`].
    Source(String),
    /// A folder of exported traces (`trajectory_source`).
    Trajectory,
}

/// A folder added on Tools, a row of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedFolder {
    pub kind: FolderKind,
    pub path: String,
    /// None until answered; a trajectory folder starts watched.
    pub watched: Option<bool>,
}
impl AddedFolder {
    pub fn new(kind: FolderKind, path: impl Into<String>) -> Self {
        let watched = (kind == FolderKind::Trajectory).then_some(true);
        Self {
            kind,
            path: path.into(),
            watched,
        }
    }
}

/// What a discovered or described store reports, the part Tools reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub source: String,
    pub path: String,
    pub exists: bool,
    pub session_count: u64,
    pub total_bytes: Option<u64>,
}
impl From<&trace_commons_contributor::source::discovery::SourceCandidate> for Found {
    fn from(c: &trace_commons_contributor::source::discovery::SourceCandidate) -> Self {
        Self {
            source: c.source.clone(),
            path: c.path.to_string_lossy().into_owned(),
            exists: c.exists,
            session_count: c.session_count,
            total_bytes: c.total_bytes,
        }
    }
}

/// What a folder picked with "add your tool" turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added(AddedFolder),
    /// More than one kind matched: the person picks which, or Neither.
    Ask {
        path: String,
        kinds: Vec<FolderKind>,
    },
    /// Nothing this build can declare matched. Nothing is added.
    Refused,
}

/// `discovery::describe_folder`'s rows as an outcome (`ToolsScreenLayout.outcome`).
pub fn add_outcome(path: &str, matches: &[Found]) -> AddOutcome {
    let offerable: Vec<FolderKind> = matches
        .iter()
        .filter_map(|m| {
            if m.source == SOURCE_TRAJECTORY {
                Some(FolderKind::Trajectory)
            } else if TOOL_ORDER.contains(&m.source.as_str()) {
                Some(FolderKind::Source(m.source.clone()))
            } else {
                None
            }
        })
        .collect();
    if matches.len() == 1 {
        if let Some(kind) = offerable.into_iter().next() {
            return AddOutcome::Added(AddedFolder::new(kind, matches[0].path.clone()));
        }
        return AddOutcome::Refused;
    }
    if matches.len() > 1 && !offerable.is_empty() {
        return AddOutcome::Ask {
            path: path.to_owned(),
            kinds: offerable,
        };
    }
    AddOutcome::Refused
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    pub answers: BTreeMap<String, ToolAnswer>,
    pub added: Vec<AddedFolder>,
    pub not_found: BTreeSet<String>,
    /// Discovery's last answer; None until it has answered.
    pub discovered: Option<Vec<Found>>,
}

impl Tools {
    /// Record discovery. A tool not here is not asked; a found tool nobody
    /// answered reads Not used, since its check starts unticked.
    pub fn record_discovery(&mut self, found: Vec<Found>) {
        self.not_found = found
            .iter()
            .filter(|f| !f.exists)
            .map(|f| f.source.clone())
            .collect();
        for f in found.iter().filter(|f| f.exists) {
            if self.row_answer(&f.source) == ToolAnswer::Undecided {
                self.answers.insert(f.source.clone(), ToolAnswer::Off);
            }
        }
        self.discovered = Some(found);
    }
    pub fn row_answer(&self, source: &str) -> ToolAnswer {
        self.answers.get(source).cloned().unwrap_or_default()
    }
    /// The found tools a check is offered for, in ring order.
    pub fn listed(&self) -> Vec<Found> {
        let found = self.discovered.as_deref().unwrap_or_default();
        TOOL_ORDER
            .iter()
            .filter_map(|tool| found.iter().find(|f| f.source == *tool && f.exists))
            .cloned()
            .collect()
    }
    pub fn is_watched(&self, source: &str) -> bool {
        matches!(self.row_answer(source), ToolAnswer::Watch(_))
    }
    /// Tick or untick a found tool's check: ticked watches its found folder.
    pub fn set_watched(&mut self, found: &Found, on: bool) {
        let answer = if on {
            ToolAnswer::Watch(found.path.clone())
        } else {
            ToolAnswer::Off
        };
        self.answers.insert(found.source.clone(), answer);
    }
    /// One folder is one thing: a folder at the same path is replaced, and a
    /// second trajectory folder replaces the first.
    pub fn add(&mut self, folder: AddedFolder) {
        self.added.retain(|f| {
            f.path != folder.path
                && !(folder.kind == FolderKind::Trajectory && f.kind == FolderKind::Trajectory)
        });
        self.added.push(folder);
    }
    pub fn answer_added(&mut self, path: &str, watched: Option<bool>) {
        if let Some(f) = self.added.iter_mut().find(|f| f.path == path) {
            f.watched = watched;
        }
    }
    pub fn withdraw_trajectory(&mut self) {
        self.added.retain(|f| f.kind != FolderKind::Trajectory);
    }
    /// Tools two rows both watch; the daemon watches one folder per tool.
    pub fn watched_twice(&self) -> BTreeSet<String> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for (kind, answer) in &self.answers {
            if matches!(answer, ToolAnswer::Watch(_)) {
                *counts.entry(kind).or_default() += 1;
            }
        }
        for f in self.added.iter().filter(|f| f.watched == Some(true)) {
            if let FolderKind::Source(kind) = &f.kind {
                *counts.entry(kind).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(k, _)| k.to_owned())
            .collect()
    }
    /// The answers and added folders as one declaration, keyed by source.
    /// A missing Claude Code or Codex nobody answered is declared off.
    pub fn declarations(&self) -> BTreeMap<String, ToolAnswer> {
        let mut roots = self.answers.clone();
        for kind in [SOURCE_CLAUDE_CODE, SOURCE_CODEX] {
            if self.not_found.contains(kind)
                && roots.get(kind).is_none_or(|a| *a == ToolAnswer::Undecided)
            {
                roots.insert(kind.to_owned(), ToolAnswer::Off);
            }
        }
        for f in self.added.iter().filter(|f| f.watched == Some(true)) {
            let key = match &f.kind {
                FolderKind::Source(kind) => kind.clone(),
                FolderKind::Trajectory => SOURCE_TRAJECTORY.to_owned(),
            };
            roots.insert(key, ToolAnswer::Watch(f.path.clone()));
        }
        roots.retain(|_, a| *a != ToolAnswer::Undecided);
        roots
    }
    pub fn watches_anything(&self) -> bool {
        self.declarations()
            .values()
            .any(|a| matches!(a, ToolAnswer::Watch(_)))
    }
    /// The watched tools in ring order (Uses, Traces and Customize read it).
    pub fn watched_kinds(&self) -> Vec<&'static str> {
        let roots = self.declarations();
        TOOL_ORDER
            .iter()
            .copied()
            .filter(|k| matches!(roots.get(*k), Some(ToolAnswer::Watch(_))))
            .collect()
    }
    /// The `set_settings` declaration, or None while Claude Code or Codex
    /// is undeclared: the daemon would not start with it.
    pub fn session_roots(&self) -> Option<super::Roots> {
        let declared = self.declarations();
        if !declared.contains_key(SOURCE_CLAUDE_CODE) || !declared.contains_key(SOURCE_CODEX) {
            return None;
        }
        let mut roots = super::Roots::new();
        for (source, answer) in declared {
            let key = trace_commons_contributor::daemon::settings::source_settings_key(&source)?;
            let value: Value = match answer {
                ToolAnswer::Off => json!({"mode":"off"}),
                ToolAnswer::Watch(path) => json!({"mode":"watch","path":path}),
                ToolAnswer::Undecided => continue,
            };
            roots.insert(key.to_owned(), value);
        }
        Some(roots)
    }
    /// Discovery's count of the watched tools' traces; None until it answered.
    pub fn trace_count(&self) -> Option<u64> {
        let found = self.discovered.as_ref()?;
        Some(
            self.watched_kinds()
                .iter()
                .filter_map(|k| found.iter().find(|f| f.source == *k && f.exists))
                .map(|f| f.session_count)
                .fold(0u64, u64::saturating_add),
        )
    }
    pub fn total_bytes(&self) -> u64 {
        let Some(found) = self.discovered.as_ref() else {
            return 0;
        };
        self.watched_kinds()
            .iter()
            .filter_map(|k| found.iter().find(|f| f.source == *k && f.exists))
            .filter_map(|f| f.total_bytes)
            .fold(0u64, u64::saturating_add)
    }
    /// The found tools Private AI can connect (`PrivateAIScreenLayout.routable`).
    pub fn connectable(&self) -> Vec<Found> {
        const ROUTABLE: [&str; 3] = [SOURCE_CLAUDE_CODE, SOURCE_CODEX, SOURCE_OPENCODE];
        self.listed()
            .into_iter()
            .filter(|f| ROUTABLE.contains(&f.source.as_str()))
            .collect()
    }
    /// How many tools discovery found here.
    pub fn found_count(&self) -> usize {
        self.discovered
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|f| f.exists)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str, exists: bool, count: u64) -> Found {
        Found {
            source: source.into(),
            path: format!("/home/x/{source}"),
            exists,
            session_count: count,
            total_bytes: Some(count * 10),
        }
    }

    fn discovered() -> Tools {
        let mut tools = Tools::default();
        tools.record_discovery(vec![
            found(SOURCE_CLAUDE_CODE, true, 3),
            found(SOURCE_CODEX, false, 0),
            found(SOURCE_OPENCODE, true, 2),
        ]);
        tools
    }

    #[test]
    fn checks_start_off_and_unticked_is_not_used() {
        let tools = discovered();
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Off);
        assert_eq!(tools.row_answer(SOURCE_OPENCODE), ToolAnswer::Off);
        // A tool not found is not asked.
        assert_eq!(tools.row_answer(SOURCE_CODEX), ToolAnswer::Undecided);
        assert_eq!(
            tools
                .listed()
                .iter()
                .map(|f| f.source.as_str())
                .collect::<Vec<_>>(),
            [SOURCE_CLAUDE_CODE, SOURCE_OPENCODE]
        );
        assert!(!tools.watches_anything());
        // Declared, but nothing watched: the daemon would read nothing.
        let roots = tools.session_roots().unwrap();
        assert_eq!(roots["claude_source"], json!({"mode":"off"}));
        assert_eq!(roots["codex_source"], json!({"mode":"off"}));
    }

    #[test]
    fn ticking_a_tool_watches_its_found_folder() {
        let mut tools = discovered();
        let claude = tools.listed()[0].clone();
        tools.set_watched(&claude, true);
        assert!(tools.watches_anything());
        assert_eq!(tools.watched_kinds(), [SOURCE_CLAUDE_CODE]);
        assert_eq!(
            tools.session_roots().unwrap()["claude_source"],
            json!({"mode":"watch","path":"/home/x/claude-code"})
        );
        assert_eq!(tools.trace_count(), Some(3));
        assert_eq!(tools.total_bytes(), 30);
        tools.set_watched(&claude, false);
        assert!(!tools.watches_anything());
    }

    #[test]
    fn a_tool_watched_in_two_rows_holds_continue() {
        let mut tools = discovered();
        let claude = tools.listed()[0].clone();
        tools.set_watched(&claude, true);
        tools.add(AddedFolder::new(
            FolderKind::Source(SOURCE_CLAUDE_CODE.into()),
            "/moved/claude",
        ));
        // An added tool folder starts unanswered.
        assert!(tools.watched_twice().is_empty());
        tools.answer_added("/moved/claude", Some(true));
        assert_eq!(
            tools.watched_twice().into_iter().collect::<Vec<_>>(),
            [SOURCE_CLAUDE_CODE]
        );
        tools.set_watched(&claude, false);
        assert!(tools.watched_twice().is_empty());
        assert_eq!(
            tools.session_roots().unwrap()["claude_source"],
            json!({"mode":"watch","path":"/moved/claude"})
        );
    }

    #[test]
    fn added_folders_replace_by_path_and_one_trajectory() {
        let mut tools = discovered();
        tools.add(AddedFolder::new(FolderKind::Trajectory, "/a"));
        tools.add(AddedFolder::new(FolderKind::Trajectory, "/b"));
        assert_eq!(tools.added.len(), 1);
        assert_eq!(tools.added[0].watched, Some(true));
        assert!(tools.watches_anything());
        assert_eq!(
            tools.session_roots().unwrap()["trajectory_source"],
            json!({"mode":"watch","path":"/b"})
        );
        tools.add(AddedFolder::new(
            FolderKind::Source(SOURCE_OPENCODE.into()),
            "/b",
        ));
        assert_eq!(tools.added.len(), 1);
        tools.withdraw_trajectory();
        assert_eq!(tools.added.len(), 1);
    }

    #[test]
    fn a_picked_folder_is_added_asked_or_refused() {
        let one = [found(SOURCE_OPENCODE, true, 1)];
        assert!(matches!(add_outcome("/p", &one), AddOutcome::Added(f) if f.watched.is_none()));
        let two = [
            found(SOURCE_OPENCODE, true, 1),
            found(SOURCE_TRAJECTORY, true, 1),
        ];
        assert_eq!(
            add_outcome("/p", &two),
            AddOutcome::Ask {
                path: "/p".into(),
                kinds: vec![
                    FolderKind::Source(SOURCE_OPENCODE.into()),
                    FolderKind::Trajectory
                ]
            }
        );
        assert_eq!(add_outcome("/p", &[]), AddOutcome::Refused);
        assert_eq!(
            add_outcome("/p", &[found("unknown", true, 1)]),
            AddOutcome::Refused
        );
    }

    #[test]
    fn without_discovery_nothing_is_declared() {
        assert!(Tools::default().session_roots().is_none());
        assert_eq!(Tools::default().trace_count(), None);
    }

    #[test]
    fn private_ai_counts_routable_found_tools_of_all_found() {
        let tools = discovered();
        assert_eq!(tools.connectable().len(), 2);
        assert_eq!(tools.found_count(), 2);
    }
}
