//! The Traces tree's per-tool aggregate, as the Traces map draws it.
//!
//! A native port of `TracesTree.build` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/TracesTree.swift`, the parts
//! the map reads: every folder the core reports (ignored ones left out),
//! newest waiting session first, and each tool with the folders most of
//! whose sessions came from it. Built only from what the core reports
//! (`list_pending`, `list_projects`, `get_settings`); nothing is estimated.
//! The tree pane itself is still the Queue page's own list.
//!
//! Deliberate platform difference: folders with nothing waiting are ordered
//! by a plain comparison of their names, where the Mac uses Foundation's
//! `localizedStandardCompare` (numbers in names compared as numbers). GTK
//! has no such comparison without a new dependency.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};

use crate::model::{Project, QueueEntry, Settings};

/// The tools this build knows, in the Mac's `SourceKind.allCases` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceKind {
    ClaudeCode,
    Codex,
    GeminiCli,
    Cline,
    Opencode,
}

impl SourceKind {
    pub const ALL: [SourceKind; 5] = [
        SourceKind::ClaudeCode,
        SourceKind::Codex,
        SourceKind::GeminiCli,
        SourceKind::Cline,
        SourceKind::Opencode,
    ];

    /// The adapter's wire name: a queue entry's `source`.
    pub fn wire(self) -> &'static str {
        match self {
            SourceKind::ClaudeCode => "claude-code",
            SourceKind::Codex => "codex",
            SourceKind::GeminiCli => "gemini-cli",
            SourceKind::Cline => "cline",
            SourceKind::Opencode => "opencode",
        }
    }

    pub fn from_wire(wire: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.wire() == wire)
    }

    /// The tool's name, as `QueueEntry::agent_label` draws it (a test
    /// holds the two together).
    pub fn display_name(self) -> &'static str {
        match self {
            SourceKind::ClaudeCode => "Claude Code",
            SourceKind::Codex => "Codex",
            SourceKind::GeminiCli => "Gemini CLI",
            SourceKind::Cline => "Cline",
            SourceKind::Opencode => "OpenCode",
        }
    }

    /// `<tool>_source_mode` in `get_settings`. The settings keys name the
    /// tool, not the adapter.
    fn source_mode(self, settings: &Settings) -> &str {
        match self {
            SourceKind::ClaudeCode => &settings.claude_source_mode,
            SourceKind::Codex => &settings.codex_source_mode,
            SourceKind::GeminiCli => &settings.gemini_source_mode,
            SourceKind::Cline => &settings.cline_source_mode,
            SourceKind::Opencode => &settings.opencode_source_mode,
        }
    }

    /// Whether the core reads this tool from its usual folder while it is
    /// undeclared (`unset_scans_conventional`): asked of the core, never
    /// typed here.
    pub fn scans_when_unset(self) -> bool {
        trace_commons_contributor::source::undeclared_scans_conventional(self.wire())
    }
}

/// A tool's source declaration. `Unknown` is settings that could not be
/// read: not an answer, and never drawn as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceMode {
    Watch,
    Off,
    Unset,
    Unknown,
}

impl SourceMode {
    pub fn from_wire(raw: &str) -> Self {
        match raw {
            "watch" => SourceMode::Watch,
            "off" => SourceMode::Off,
            _ => SourceMode::Unset,
        }
    }
}

/// What the daemon does with a folder's sessions, by its wire name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectMode {
    Ask,
    AutoUpload,
    Ignore,
}

impl ProjectMode {
    /// `None` for a mode this build does not know: drawn as unlisted.
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "notify_only" => Some(ProjectMode::Ask),
            "auto_upload" => Some(ProjectMode::AutoUpload),
            "ignore" => Some(ProjectMode::Ignore),
            _ => None,
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            ProjectMode::Ask => "notify_only",
            ProjectMode::AutoUpload => "auto_upload",
            ProjectMode::Ignore => "ignore",
        }
    }

    /// The mode's one name, the core's (`project_copy::FOLDER_MODE_LABELS`).
    pub fn label(self) -> &'static str {
        trace_commons_contributor::project_copy::folder_mode_label(self.wire()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FolderNode {
    pub id: String,
    pub label: String,
    /// `None` for a folder the queue names but `list_projects` does not.
    pub mode: Option<ProjectMode>,
    pub sessions: Vec<QueueEntry>,
    /// The folder's path from `list_projects`; `None` for one it does not
    /// list.
    pub path: Option<String>,
}

impl FolderNode {
    pub fn new(id: &str, label: &str, mode: Option<ProjectMode>) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            mode,
            ..Self::default()
        }
    }

    /// When its newest waiting session started, or was found if no start
    /// was recorded; `None` with nothing waiting.
    fn most_recent(&self) -> Option<DateTime<Utc>> {
        self.sessions
            .iter()
            .filter_map(|session| session.started_at.or(session.discovered_at))
            .max()
    }
}

impl PartialEq for FolderNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.label == other.label
            && self.mode == other.mode
            && self.path == other.path
            && self.sessions.len() == other.sessions.len()
            && self
                .sessions
                .iter()
                .zip(&other.sessions)
                .all(|(a, b)| a.entry_id == b.entry_id && a.source == b.source)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolNode {
    pub kind: SourceKind,
    pub mode: SourceMode,
    pub folders: Vec<FolderNode>,
}

impl ToolNode {
    pub fn new(kind: SourceKind, mode: SourceMode, folders: Vec<FolderNode>) -> Self {
        Self {
            kind,
            mode,
            folders,
        }
    }

    pub fn id(&self) -> &'static str {
        self.kind.wire()
    }

    /// The sessions waiting in this tool's folders.
    pub fn waiting(&self) -> usize {
        self.folders
            .iter()
            .map(|folder| folder.sessions.len())
            .sum()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracesTree {
    pub tools: Vec<ToolNode>,
    /// Folders no session places under a tool yet.
    pub unplaced: Vec<FolderNode>,
    /// Every folder the core reports, ignored ones left out, newest
    /// waiting session first.
    pub folders: Vec<FolderNode>,
}

impl TracesTree {
    /// Whether a tool is drawn: one that is declared, or has folders, is.
    /// Unset or unknown, it is drawn when the core reads it from its usual
    /// folder anyway, since leaving it out would say it is not watched.
    pub fn draws_tool(kind: SourceKind, mode: SourceMode, has_folders: bool) -> bool {
        match mode {
            SourceMode::Watch | SourceMode::Off => true,
            SourceMode::Unset | SourceMode::Unknown => has_folders || kind.scans_when_unset(),
        }
    }

    /// The tree from the core's three reads. `settings` is `None` when
    /// `get_settings` has not answered, and every tool is then unknown.
    pub fn build(
        entries: &[QueueEntry],
        projects: &[Project],
        settings: Option<&Settings>,
    ) -> TracesTree {
        let mut folders: HashMap<String, FolderNode> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut add = |node: FolderNode, folders: &mut HashMap<String, FolderNode>| {
            if !folders.contains_key(&node.id) {
                order.push(node.id.clone());
                folders.insert(node.id.clone(), node);
            }
        };
        // The core's rows first, so a folder takes its listed name and mode;
        // the bucket is drawn under its shared name, never its slug.
        for project in projects {
            let label = if project.is_unresolved_bucket {
                crate::copy::ONBOARD_WATCH_UNKNOWN_LABEL.to_string()
            } else {
                project.project_label.clone()
            };
            add(
                FolderNode {
                    id: project.project_id.clone(),
                    label,
                    mode: ProjectMode::from_wire(&project.mode),
                    sessions: Vec::new(),
                    path: Some(project.project_path.clone()),
                },
                &mut folders,
            );
        }
        for entry in entries.iter().filter(|entry| entry.state == "pending") {
            add(
                FolderNode::new(&entry.project_id, &entry.project_label, None),
                &mut folders,
            );
            if let Some(folder) = folders.get_mut(&entry.project_id) {
                folder.sessions.push(entry.clone());
            }
        }

        let mut by_tool: BTreeMap<SourceKind, Vec<FolderNode>> = BTreeMap::new();
        let mut unplaced = Vec::new();
        let mut drawn = Vec::new();
        for id in &order {
            let Some(mut node) = folders.remove(id) else {
                continue;
            };
            if node.mode == Some(ProjectMode::Ignore) {
                continue;
            }
            node.sessions
                .sort_by(|a, b| b.started_at.cmp(&a.started_at));
            drawn.push(node.clone());
            match Self::majority_tool(&node.sessions) {
                Some(kind) => by_tool.entry(kind).or_default().push(node),
                None => unplaced.push(node),
            }
        }

        let mut tools: Vec<ToolNode> = SourceKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let mode = settings.map_or(SourceMode::Unknown, |settings| {
                    SourceMode::from_wire(kind.source_mode(settings))
                });
                let mut nodes = by_tool.remove(&kind).unwrap_or_default();
                nodes.sort_by(|a, b| b.sessions.len().cmp(&a.sessions.len()));
                Self::draws_tool(kind, mode, !nodes.is_empty())
                    .then(|| ToolNode::new(kind, mode, nodes))
            })
            .collect();
        tools.sort_by(|a, b| b.waiting().cmp(&a.waiting()));

        TracesTree {
            tools,
            unplaced,
            folders: Self::ordered(drawn),
        }
    }

    /// Newest waiting session first; folders with nothing waiting after
    /// them; ties by name, then by id.
    pub fn ordered(mut folders: Vec<FolderNode>) -> Vec<FolderNode> {
        folders.sort_by(|a, b| match (a.most_recent(), b.most_recent()) {
            (Some(x), Some(y)) if x != y => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a
                .label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then_with(|| a.id.cmp(&b.id)),
        });
        folders
    }

    /// The tool most of a folder's sessions came from; `None` with none, or
    /// when the sessions name no tool this build knows. Ties go to the
    /// tool whose wire name sorts first.
    pub fn majority_tool(sessions: &[QueueEntry]) -> Option<SourceKind> {
        let mut counts: BTreeMap<SourceKind, usize> = BTreeMap::new();
        for session in sessions {
            let kind = session
                .declared_source
                .as_deref()
                .and_then(SourceKind::from_wire)
                .or_else(|| SourceKind::from_wire(&session.source));
            if let Some(kind) = kind {
                *counts.entry(kind).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by(|(a_kind, a), (b_kind, b)| {
                a.cmp(b).then_with(|| b_kind.wire().cmp(a_kind.wire()))
            })
            .map(|(kind, _)| kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, project: &str, source: &str) -> QueueEntry {
        QueueEntry {
            entry_id: id.into(),
            project_id: project.into(),
            project_label: project.into(),
            source: source.into(),
            state: "pending".into(),
            ..QueueEntry::default()
        }
    }

    #[test]
    fn a_folder_sits_under_the_tool_most_of_its_sessions_came_from() {
        let entries = vec![
            entry("a", "p1", "codex"),
            entry("b", "p1", "codex"),
            entry("c", "p1", "claude-code"),
            entry("d", "p2", "something-new"),
        ];
        let tree = TracesTree::build(&entries, &[], None);
        let codex = tree
            .tools
            .iter()
            .find(|t| t.kind == SourceKind::Codex)
            .unwrap();
        assert_eq!(codex.folders.len(), 1);
        assert_eq!(codex.folders[0].id, "p1");
        assert_eq!(tree.unplaced.len(), 1, "an unknown tool places nothing");
        assert_eq!(tree.unplaced[0].id, "p2");
        assert_eq!(tree.folders.len(), 2);
    }

    #[test]
    fn unread_settings_are_unknown_and_never_unset() {
        let tree = TracesTree::build(&[entry("a", "p1", "codex")], &[], None);
        assert!(tree.tools.iter().all(|t| t.mode == SourceMode::Unknown));
    }

    #[test]
    fn an_undeclared_tool_that_opens_nothing_is_drawn_only_with_something_waiting() {
        for kind in SourceKind::ALL {
            assert!(TracesTree::draws_tool(kind, SourceMode::Watch, false));
            assert!(TracesTree::draws_tool(kind, SourceMode::Off, false));
            assert!(TracesTree::draws_tool(kind, SourceMode::Unset, true));
            assert_eq!(
                TracesTree::draws_tool(kind, SourceMode::Unset, false),
                kind.scans_when_unset(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn ignored_folders_are_left_out_and_unlisted_modes_are_none() {
        let project = |id: &str, mode: &str| -> Project {
            serde_json::from_value(serde_json::json!({
                "project_id": id, "project_label": id, "mode": mode
            }))
            .unwrap()
        };
        let tree = TracesTree::build(
            &[entry("a", "q", "codex")],
            &[project("p", "ignore"), project("r", "auto_upload")],
            None,
        );
        let ids: Vec<_> = tree.folders.iter().map(|f| f.id.as_str()).collect();
        assert!(!ids.contains(&"p"));
        assert_eq!(
            tree.folders.iter().find(|f| f.id == "r").unwrap().mode,
            Some(ProjectMode::AutoUpload)
        );
        assert_eq!(
            tree.folders.iter().find(|f| f.id == "q").unwrap().mode,
            None
        );
        // Folders with something waiting come first.
        assert_eq!(tree.folders[0].id, "q");
    }

    #[test]
    fn every_kind_reads_back_from_its_wire_name() {
        for kind in SourceKind::ALL {
            assert_eq!(SourceKind::from_wire(kind.wire()), Some(kind));
            assert!(!kind.display_name().is_empty());
            // The queue's names, except OpenCode, which `agent_label` draws
            // as its raw wire name and the Mac map names.
            if kind != SourceKind::Opencode {
                let entry = QueueEntry {
                    source: kind.wire().into(),
                    ..QueueEntry::default()
                };
                assert_eq!(entry.agent_label(), kind.display_name());
            }
        }
        for mode in [
            ProjectMode::Ask,
            ProjectMode::AutoUpload,
            ProjectMode::Ignore,
        ] {
            assert_eq!(ProjectMode::from_wire(mode.wire()), Some(mode));
            assert!(!mode.label().is_empty());
        }
    }
}
