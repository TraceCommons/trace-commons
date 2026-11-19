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
    /// Watched, in the folder the attached daemon already holds for it
    /// (`get_settings` says "watch" and never where). Nothing is sent for
    /// it, so a commit never replaces that folder.
    Held,
}

/// The tools the attached daemon already watches, from its `get_settings`
/// answer: each `*_source_mode` that is "watch".
pub fn held_watches(settings: &Value) -> BTreeSet<String> {
    TOOL_ORDER
        .iter()
        .filter(|source| {
            trace_commons_contributor::daemon::settings::source_settings_key(source)
                .and_then(|key| settings.get(format!("{key}_mode")))
                .and_then(Value::as_str)
                == Some("watch")
        })
        .map(|source| (*source).to_owned())
        .collect()
}

/// The tool whose declaration `settings_key` is (`claude_source` is
/// Claude Code); `None` for a key no tool row declares.
pub fn tool_for_settings_key(settings_key: &str) -> Option<&'static str> {
    TOOL_ORDER.iter().copied().find(|source| {
        trace_commons_contributor::daemon::settings::source_settings_key(source)
            == Some(settings_key)
    })
}

/// What the attached daemon's `get_settings` answer says about the tools:
/// which it watches ([`held_watches`]), and how many times each one's
/// declaration has been written ([`declaration_revisions`]). Never where.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watches {
    pub held: BTreeSet<String>,
    pub revisions: BTreeMap<String, u64>,
}

impl Watches {
    pub fn read(settings: &Value) -> Self {
        Self {
            held: held_watches(settings),
            revisions: declaration_revisions(settings),
        }
    }
}

/// Each tool's declaration revision in the daemon's `get_settings` answer
/// (`source_revisions`, by settings key): absent for a declaration never
/// written through the daemon, and from a daemon that predates the count.
pub fn declaration_revisions(settings: &Value) -> BTreeMap<String, u64> {
    let Some(revisions) = settings.get("source_revisions") else {
        return BTreeMap::new();
    };
    TOOL_ORDER
        .iter()
        .filter_map(|source| {
            let key = trace_commons_contributor::daemon::settings::source_settings_key(source)?;
            let revision = revisions.get(key)?.as_u64()?;
            Some(((*source).to_owned(), revision))
        })
        .collect()
}

/// A refusal shown on Tools: the tool its old-portal line named, and what
/// the daemon's last answer before it said ([`Watches`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub tool: &'static str,
    pub before: Watches,
}

/// Whether `refusal` no longer holds by the daemon's answer `now`: the
/// command was run. Either the daemon newly holds the tool, or it held the
/// tool already and the tool's declaration has been written since -- the
/// command for another folder of a tool already watched landed (#2171
/// review 2, finding 4). Another tool newly held or written, an answer
/// that changed nothing for the tool, or a daemon that reports no
/// revisions leaves the refusal shown.
pub fn refusal_ended(refusal: Option<&Refusal>, now: &Watches) -> bool {
    refusal.is_some_and(|refusal| {
        let tool = refusal.tool;
        let before = &refusal.before;
        now.held.contains(tool)
            && (!before.held.contains(tool)
                || now
                    .revisions
                    .get(tool)
                    .is_some_and(|revision| before.revisions.get(tool) != Some(revision)))
    })
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
    /// The tools the attached daemon already watches ([`held_watches`]).
    pub held: BTreeSet<String>,
    /// The tools whose check the person set in this window.
    pub touched: BTreeSet<String>,
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
    /// The found tools a check is offered for, in ring order, and the
    /// tools the daemon already watches that discovery here did not find
    /// (a folder this window cannot see), with no folder of their own.
    pub fn listed(&self) -> Vec<Found> {
        let found = self.discovered.as_deref().unwrap_or_default();
        TOOL_ORDER
            .iter()
            .filter_map(|tool| {
                found
                    .iter()
                    .find(|f| f.source == *tool && f.exists)
                    .cloned()
                    .or_else(|| {
                        self.held.contains(*tool).then(|| Found {
                            source: (*tool).to_owned(),
                            path: String::new(),
                            exists: true,
                            session_count: 0,
                            total_bytes: None,
                        })
                    })
            })
            .collect()
    }
    pub fn is_watched(&self, source: &str) -> bool {
        matches!(
            self.row_answer(source),
            ToolAnswer::Watch(_) | ToolAnswer::Held
        )
    }
    /// Tick or untick a found tool's check: ticked watches its found folder,
    /// or, for a tool the daemon already watches, keeps the daemon's.
    pub fn set_watched(&mut self, found: &Found, on: bool) {
        let answer = if !on {
            ToolAnswer::Off
        } else if self.held.contains(&found.source) {
            ToolAnswer::Held
        } else {
            ToolAnswer::Watch(found.path.clone())
        };
        self.touched.insert(found.source.clone());
        self.answers.insert(found.source.clone(), answer);
    }
    /// Record which tools the attached daemon watches ([`held_watches`]).
    /// A tool it newly holds reads ticked, in its folder, unless the person
    /// turned it off here; a folder added here for that tool stops being
    /// watched, since the daemon reads one folder per tool and holds one.
    /// A tool it no longer holds reads off.
    /// Returns the tools it newly holds.
    pub fn record_held(&mut self, held: BTreeSet<String>) -> BTreeSet<String> {
        let newly: BTreeSet<String> = held.difference(&self.held).cloned().collect();
        for source in &newly {
            self.hold(source);
        }
        for source in self.held.difference(&held) {
            if self.row_answer(source) == ToolAnswer::Held {
                self.answers.insert(source.clone(), ToolAnswer::Off);
            }
        }
        self.held = held;
        newly
    }
    /// The daemon's declaration for `source`, a tool it already held, has
    /// been written since a folder for it was picked here: the command for
    /// that folder landed, so the row reads as the daemon's own again, as
    /// for a tool it newly holds ([`Self::record_held`]).
    pub fn record_landed(&mut self, source: &str) {
        if self.held.contains(source) {
            self.hold(source);
        }
    }
    /// `source`'s row reads as the daemon's folder, unless the person
    /// turned it off here, and a folder added here for it stops being
    /// watched.
    fn hold(&mut self, source: &str) {
        let turned_off =
            self.touched.contains(source) && self.row_answer(source) == ToolAnswer::Off;
        if !turned_off {
            self.answers.insert(source.to_owned(), ToolAnswer::Held);
        }
        for folder in self.added.iter_mut() {
            if folder.kind == FolderKind::Source(source.to_owned()) {
                folder.watched = None;
            }
        }
    }
    /// Whether the declaration for `settings_key` is the daemon's own,
    /// which no commit sends.
    pub fn holds(&self, settings_key: &str) -> bool {
        self.declarations().iter().any(|(source, answer)| {
            *answer == ToolAnswer::Held
                && trace_commons_contributor::daemon::settings::source_settings_key(source)
                    == Some(settings_key)
        })
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
            if matches!(answer, ToolAnswer::Watch(_) | ToolAnswer::Held) {
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
    /// Tools' Continue: something watched, no tool in two rows, and a
    /// declaration the daemon would take.
    pub fn can_continue(&self) -> bool {
        self.watches_anything() && self.watched_twice().is_empty() && self.session_roots().is_some()
    }
    pub fn watches_anything(&self) -> bool {
        self.declarations()
            .values()
            .any(|a| matches!(a, ToolAnswer::Watch(_) | ToolAnswer::Held))
    }
    /// The watched tools in ring order (Uses, Traces and Customize read it).
    pub fn watched_kinds(&self) -> Vec<&'static str> {
        let roots = self.declarations();
        TOOL_ORDER
            .iter()
            .copied()
            .filter(|k| matches!(roots.get(*k), Some(ToolAnswer::Watch(_) | ToolAnswer::Held)))
            .collect()
    }
    /// The `set_settings` declaration, or None while Claude Code or Codex
    /// is undeclared: the daemon would not start with it. A tool the
    /// daemon already watches counts as declared and is left out.
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
                // The daemon's own folder: nothing to send.
                ToolAnswer::Held | ToolAnswer::Undecided => continue,
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
    fn the_declaration_is_the_daemons_own_wire_shape() {
        use trace_commons_contributor::daemon::settings::SourceDeclaration;
        let mut tools = discovered();
        let claude = tools.listed()[0].clone();
        tools.set_watched(&claude, true);
        let roots = tools.session_roots().unwrap();
        assert_eq!(
            roots["claude_source"],
            serde_json::to_value(SourceDeclaration::Watch {
                path: "/home/x/claude-code".into()
            })
            .unwrap()
        );
        assert_eq!(
            roots["codex_source"],
            serde_json::to_value(SourceDeclaration::Off).unwrap()
        );
    }

    fn held(sources: &[&str]) -> BTreeSet<String> {
        sources.iter().map(|s| (*s).to_owned()).collect()
    }

    /// The daemon's `get_settings` names which tools it watches, never
    /// where: `*_source_mode` "watch" is held, "off" and "unset" are not.
    #[test]
    fn the_daemons_watches_are_read_from_its_modes() {
        let settings = json!({
            "claude_source_mode": "watch",
            "codex_source_mode": "off",
            "opencode_source_mode": "unset",
            "gemini_source_mode": "watch",
            "claude_root_configured": true,
        });
        assert_eq!(
            held_watches(&settings),
            held(&[SOURCE_CLAUDE_CODE, SOURCE_GEMINI_CLI])
        );
        assert!(held_watches(&json!({})).is_empty());
    }

    /// After the old-portal command, the daemon watches the folder the
    /// command declared: Tools shows that tool ticked, Continue works, and
    /// nothing it sends replaces the folder (#2171 review, finding 3).
    #[test]
    fn a_tool_the_daemon_already_watches_is_ticked_and_never_overwritten() {
        let mut tools = discovered();
        tools.record_held(held(&[SOURCE_CLAUDE_CODE]));
        assert!(tools.is_watched(SOURCE_CLAUDE_CODE));
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Held);
        assert!(tools.can_continue());
        assert_eq!(tools.watched_kinds(), [SOURCE_CLAUDE_CODE]);
        // Nothing is sent for it: the daemon keeps its folder. Codex, not
        // found and unanswered, is still declared off.
        let roots = tools.session_roots().unwrap();
        assert!(!roots.contains_key("claude_source"), "{roots:?}");
        assert_eq!(roots["codex_source"], json!({"mode":"off"}));
        assert!(tools.holds("claude_source"));
        assert!(!tools.holds("codex_source"));
        // Ticking it again keeps the daemon's folder; unticking is off.
        let claude = tools.listed()[0].clone();
        tools.set_watched(&claude, false);
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Off);
        assert_eq!(
            tools.session_roots().unwrap()["claude_source"],
            json!({"mode":"off"})
        );
        tools.set_watched(&claude, true);
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Held);
        assert!(!tools.session_roots().unwrap().contains_key("claude_source"));
    }

    /// A tool ticked before the daemon's answer came is still the daemon's
    /// folder; one the person turned off stays off. A folder added for a
    /// tool the daemon then holds stops being watched here: one folder per
    /// tool, and the daemon holds it.
    #[test]
    fn the_daemons_watch_wins_over_a_tick_but_not_over_an_off() {
        let mut tools = discovered();
        let claude = tools.listed()[0].clone();
        tools.set_watched(&claude, true);
        let opencode = tools.listed()[1].clone();
        tools.set_watched(&opencode, false);
        tools.add(AddedFolder::new(
            FolderKind::Source(SOURCE_CODEX.into()),
            "/run/user/1000/doc/a1/projects",
        ));
        tools.answer_added("/run/user/1000/doc/a1/projects", Some(true));
        tools.record_held(held(&[SOURCE_CLAUDE_CODE, SOURCE_OPENCODE, SOURCE_CODEX]));
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Held);
        assert_eq!(tools.row_answer(SOURCE_OPENCODE), ToolAnswer::Off);
        assert_eq!(tools.added[0].watched, None);
        assert!(tools.watched_twice().is_empty());
        let roots = tools.session_roots().unwrap();
        assert!(!roots.contains_key("claude_source"));
        assert!(!roots.contains_key("codex_source"));
        assert_eq!(roots["opencode_source"], json!({"mode":"off"}));
        assert!(tools.can_continue());
        // Read again, the same answer changes nothing.
        let before = tools.clone();
        tools.record_held(held(&[SOURCE_CLAUDE_CODE, SOURCE_OPENCODE, SOURCE_CODEX]));
        assert_eq!(tools, before);
        // The daemon no longer holds one: that tool reads off again.
        tools.record_held(held(&[SOURCE_CODEX]));
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Off);
        assert!(tools.is_watched(SOURCE_CODEX));
        assert!(tools.can_continue());
    }

    /// A refusal whose old-portal line names a tool ends when the daemon
    /// newly holds that tool (the command was run): Tools hides the failure
    /// line and the command. Another tool newly held, a read that holds
    /// nothing new, or no refusal leaves it shown (#2171 review 2,
    /// finding 1).
    #[test]
    fn a_refusal_ends_when_the_daemon_newly_holds_its_tool() {
        assert_eq!(
            tool_for_settings_key("claude_source"),
            Some(SOURCE_CLAUDE_CODE)
        );
        assert_eq!(tool_for_settings_key("codex_source"), Some(SOURCE_CODEX));
        assert_eq!(tool_for_settings_key("trajectory_source"), None);
        assert_eq!(tool_for_settings_key("claude_root"), None);
        let watches = |sources: &[&str]| Watches {
            held: held(sources),
            revisions: BTreeMap::new(),
        };
        let refusal = Refusal {
            tool: tool_for_settings_key("claude_source").unwrap(),
            before: Watches::default(),
        };
        assert!(!refusal_ended(Some(&refusal), &watches(&[SOURCE_CODEX])));
        assert!(!refusal_ended(Some(&refusal), &Watches::default()));
        let both = watches(&[SOURCE_CODEX, SOURCE_CLAUDE_CODE]);
        assert!(refusal_ended(Some(&refusal), &both));
        assert!(!refusal_ended(None, &both));
    }

    /// A refusal for another folder of a tool the daemon already watches
    /// ends once the command lands: the tool stays held, and its
    /// declaration's revision moves. Another tool's revision moving, an
    /// answer with the same revision, or a daemon with no revisions leaves
    /// it shown (#2171 review 2, finding 4).
    #[test]
    fn a_refusal_for_a_tool_already_held_ends_when_its_declaration_is_written() {
        let settings = |revisions: Value| {
            json!({
                "claude_source_mode": "watch",
                "codex_source_mode": "watch",
                "source_revisions": revisions,
            })
        };
        let before = Watches::read(&settings(json!({"codex_source": 4})));
        assert_eq!(before.held, held(&[SOURCE_CLAUDE_CODE, SOURCE_CODEX]));
        assert_eq!(
            before.revisions,
            BTreeMap::from([(SOURCE_CODEX.to_owned(), 4)])
        );
        let refusal = Refusal {
            tool: SOURCE_CLAUDE_CODE,
            before,
        };
        assert!(!refusal_ended(Some(&refusal), &refusal.before));
        assert!(!refusal_ended(
            Some(&refusal),
            &Watches::read(&settings(json!({"codex_source": 5})))
        ));
        assert!(refusal_ended(
            Some(&refusal),
            &Watches::read(&settings(json!({"codex_source": 4, "claude_source": 1})))
        ));
        // A daemon that reports no revisions cannot say; the refusal stays.
        let older = Watches::read(&json!({
            "claude_source_mode": "watch",
            "codex_source_mode": "watch",
        }));
        assert!(older.revisions.is_empty());
        let refusal = Refusal {
            tool: SOURCE_CLAUDE_CODE,
            before: older.clone(),
        };
        assert!(!refusal_ended(Some(&refusal), &older));
    }

    /// A folder picked for a tool the daemon already holds, then declared
    /// from a terminal: once the declaration lands, Continue sends nothing
    /// for that tool, as for a tool newly held -- re-sending the folder the
    /// portal could not map would only be refused again (#2171 review 2,
    /// finding 4).
    #[test]
    fn a_landed_declaration_for_a_held_tool_is_the_daemons_own_again() {
        let mut tools = discovered();
        tools.record_held(held(&[SOURCE_CLAUDE_CODE, SOURCE_CODEX]));
        tools.add(AddedFolder::new(
            FolderKind::Source(SOURCE_CLAUDE_CODE.into()),
            "/run/user/1000/doc/abc/claude",
        ));
        tools.answer_added("/run/user/1000/doc/abc/claude", Some(true));
        assert_eq!(
            tools.session_roots().unwrap()["claude_source"],
            json!({"mode":"watch","path":"/run/user/1000/doc/abc/claude"})
        );
        tools.record_landed(SOURCE_CLAUDE_CODE);
        assert!(!tools.session_roots().unwrap().contains_key("claude_source"));
        assert_eq!(tools.row_answer(SOURCE_CLAUDE_CODE), ToolAnswer::Held);
    }

    /// A held tool this window's discovery cannot see still has its row.
    #[test]
    fn a_held_tool_discovery_did_not_find_still_has_a_row() {
        let mut tools = discovered();
        tools.record_held(held(&[SOURCE_CODEX]));
        assert_eq!(
            tools
                .listed()
                .iter()
                .map(|f| f.source.as_str())
                .collect::<Vec<_>>(),
            [SOURCE_CLAUDE_CODE, SOURCE_CODEX, SOURCE_OPENCODE]
        );
        assert!(tools.is_watched(SOURCE_CODEX));
        let roots = tools.session_roots().unwrap();
        assert!(!roots.contains_key("codex_source"), "never sent as off");
        assert_eq!(roots["claude_source"], json!({"mode":"off"}));
        // Both held: nothing to send, and Continue still works.
        tools.record_held(held(&[SOURCE_CLAUDE_CODE, SOURCE_CODEX]));
        assert_eq!(
            tools.session_roots(),
            Some(super::super::Roots::from([(
                "opencode_source".to_owned(),
                json!({"mode":"off"})
            )]))
        );
        assert!(tools.can_continue());
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
