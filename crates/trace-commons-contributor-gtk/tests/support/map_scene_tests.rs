//! `FlowMapSceneTests.swift`, ported: the Traces map draws only what the
//! core reports, against the Mac's recorded sample sets (read from
//! `macos/Sources/TCShellCore/DataContract/RecordedSamples/`, never a
//! sample set of this file's own), and every node is something a screen
//! reader can inspect.
//!
//! The Private AI scene's tests are ported too, with those of
//! `FlowMapLoadTests.swift`, `GlassParityRoundOneTests.swift` and
//! `ManagedSessionsRenderTests.swift` that build it.
//!
//! Not ported here, because they test the Inference tab, a later slice of
//! the 2026-11-16 spec:
//! `test_pricedTotalsAreUnknownWhenAnyCallIs`,
//! `test_onlyVerifiedReadsAsProof`,
//! `test_toolNamesComeFromTheAdapterOrStayRaw`,
//! `test_theStoreReadsEachMethodAlone`,
//! `test_totalsAreNeverOnePageOfMany`,
//! `test_theCallCountIsTheCoresToolCounts`,
//! `test_aCoreDownInferenceStoreDrawsNoCounts`,
//! `test_theInferenceTabSaysItsWindowFromTheData`.

use super::*;
use crate::model::{Project, QueueEntry, Settings};
use crate::ui::monitor::traces_tree::{
    FolderNode, ProjectMode, SourceKind, SourceMode, ToolNode, TracesTree,
};

const SETS: [&str; 6] = [
    "empty",
    "normalDay",
    "busyQueue",
    "heldSessions",
    "armedFolder",
    "unknownCounts",
];

fn sample(set: &str, method: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../macos/Sources/TCShellCore/DataContract/RecordedSamples")
        .join(set)
        .join(format!("{method}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("recorded sample is JSON")
}

fn tree(set: &str) -> TracesTree {
    let entries: Vec<QueueEntry> =
        serde_json::from_value(sample(set, "list_pending")["pending"].clone()).unwrap();
    let projects: Vec<Project> =
        serde_json::from_value(sample(set, "list_projects")["projects"].clone()).unwrap();
    let settings: Option<Settings> = serde_json::from_value(sample(set, "get_settings")).ok();
    TracesTree::build(&entries, &projects, settings.as_ref())
}

fn destinations(set: &str) -> ToolDestinations {
    serde_json::from_value(sample(set, "tool_destinations")).unwrap()
}

/// The gate as the core reports it for `set`, with `edit` applied to the
/// status's JSON object.
fn gate_with(
    set: &str,
    state: ScreenState,
    edit: impl FnOnce(&mut serde_json::Value),
) -> CommonsGate {
    let mut status = sample(set, "status");
    edit(&mut status);
    CommonsGate {
        state,
        status: Some(serde_json::from_value(status).unwrap()),
        destinations: Some(destinations(set)),
    }
}

fn gate(set: &str) -> CommonsGate {
    gate_with(set, ScreenState::Ready, |_| {})
}

fn traces(tree: &TracesTree, gate: &CommonsGate) -> Scene {
    Scene::traces(
        tree,
        gate,
        &Contributions::unread(),
        None,
        &BTreeMap::new(),
        SIZE,
    )
}

fn flowing(scene: &Scene) -> usize {
    scene
        .arcs
        .iter()
        .filter(|a| a.style == ArcStyle::Flowing)
        .count()
}

fn node<'a>(scene: &'a Scene, id: &str) -> Option<&'a Node> {
    scene.nodes.iter().find(|n| n.id == id)
}

fn library_kind(scene: &Scene) -> NodeKind {
    node(scene, "library").unwrap().kind
}

fn folders(count: usize) -> Vec<FolderNode> {
    (0..count)
        .map(|i| FolderNode::new(&format!("f{i}"), &format!("f{i}"), Some(ProjectMode::Ask)))
        .collect()
}

fn one_tool(folders: Vec<FolderNode>) -> TracesTree {
    TracesTree {
        tools: vec![ToolNode::new(
            SourceKind::ClaudeCode,
            SourceMode::Watch,
            folders,
        )],
        ..TracesTree::default()
    }
}

fn drawn_folders(scene: &Scene) -> usize {
    scene
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Folder(_)))
        .count()
}

#[test]
fn managed_opencode_appears_in_its_trace_card_without_claiming_contribution() {
    let mut wire = sample("normalDay", "tool_destinations");
    wire["managed_sessions"] = serde_json::json!([{
        "id": "managed-one", "tool": "opencode", "connection": "api_key",
        "provider": "openai", "model": "gpt-4.1", "collection_state": "pending", "state": "running"
    }]);
    wire["managed_copy"] = serde_json::json!({
        "title": "Managed sessions", "state_running": "Running",
        "collection_pending": "Trace collection pending", "provider_openai": "OpenAI"
    });
    let gate = CommonsGate {
        state: ScreenState::Ready,
        status: None,
        destinations: Some(serde_json::from_value(wire).unwrap()),
    };
    let scene = traces(&tree("normalDay"), &gate);
    let opencode = node(&scene, "tool:opencode").expect("OpenCode is drawn");
    assert!(opencode.detail.contains("gpt-4.1"));
    assert!(opencode.detail.contains("OpenAI"));
    assert!(opencode.detail.contains("Trace collection pending"));
    assert!(
        !scene.flows(),
        "a managed running process is not evidence of contribution or model calls"
    );
}

#[test]
fn the_map_hides_absent_tools_with_installed_evidence_but_keeps_imported_sources() {
    let wire = serde_json::json!({"tools": [
        {"tool": "claude-code", "installed": false},
        {"tool": "codex", "installed": false},
        {"tool": "opencode", "installed": true}
    ]});
    let destinations: ToolDestinations = serde_json::from_value(wire).unwrap();
    let gate = CommonsGate {
        state: ScreenState::Ready,
        status: None,
        destinations: Some(destinations),
    };
    let empty = TracesTree::default();
    let scene = traces(&empty, &gate);
    assert!(node(&scene, "tool:claude-code").is_none());
    assert!(node(&scene, "tool:codex").is_none());
    assert!(node(&scene, "tool:opencode").is_some());
    let imported = TracesTree {
        tools: vec![ToolNode::new(
            SourceKind::ClaudeCode,
            SourceMode::Watch,
            vec![],
        )],
        ..TracesTree::default()
    };
    assert!(node(&traces(&imported, &gate), "tool:claude-code").is_some());
    let unknown = traces(&empty, &CommonsGate::with_state(ScreenState::Ready));
    assert_eq!(
        unknown
            .nodes
            .iter()
            .filter(|n| n.id.starts_with("tool:"))
            .count(),
        SourceKind::ALL.len()
    );
}

/// One node per tool this build knows and per drawn folder, plus this
/// computer and the commons; every node inside the design space. The tools
/// the tree leaves out are drawn dashed, and every tool is a coin in the
/// one pile.
#[test]
fn the_traces_scene_draws_every_tool_and_folder() {
    for set in SETS {
        let tree = tree(set);
        let scene = traces(&tree, &gate(set));
        let shown = Scene::directories(&tree).len().min(FOLDERS_SHOWN);
        let tools: Vec<&Node> = scene
            .nodes
            .iter()
            .filter(|n| n.id.starts_with("tool:"))
            .collect();
        assert_eq!(
            scene.nodes.len(),
            SourceKind::ALL.len() + shown + 2,
            "{set}"
        );
        assert_eq!(
            scene.stack.as_ref().unwrap().tools,
            tools.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            "{set}"
        );
        for tool in &tree.tools {
            let drawn = tools.iter().find(|n| n.id == format!("tool:{}", tool.id()));
            assert_eq!(drawn.map(|n| n.dashed), Some(false), "{set}: {}", tool.id());
        }
        assert_eq!(
            tools.iter().filter(|n| n.dashed).count(),
            SourceKind::ALL.len() - tree.tools.len(),
            "{set}"
        );
        for node in &scene.nodes {
            assert!(
                SIZE.contains(node.at),
                "{set}: {} at {:?}",
                node.id,
                node.at
            );
        }
    }
}

/// With no tool reported installed, every tool this build knows is drawn,
/// as the Mac's own count says.
#[test]
fn without_installed_evidence_every_tool_is_a_coin() {
    let tree = tree("normalDay");
    let scene = traces(&tree, &CommonsGate::with_state(ScreenState::Ready));
    assert_eq!(
        scene
            .nodes
            .iter()
            .filter(|n| n.id.starts_with("tool:"))
            .count(),
        SourceKind::ALL.len()
    );
}

/// Every node has a name and a detail, so inspecting it never needs a
/// pointer, and ids are unique so selection is unambiguous.
#[test]
fn every_node_is_named_and_described() {
    let scene = traces(&tree("normalDay"), &gate("normalDay"));
    let ids: std::collections::HashSet<_> = scene.nodes.iter().map(|n| &n.id).collect();
    assert_eq!(ids.len(), scene.nodes.len());
    for node in &scene.nodes {
        assert!(!node.label.is_empty(), "{}", node.id);
        assert!(!node.detail.is_empty(), "{}", node.id);
    }
}

/// The arc to the commons moves for a tool with a folder set to contribute
/// automatically, when the core says that tool's sessions go there and
/// nothing stops them; it is the only thing moving.
#[test]
fn only_automatic_folders_reach_the_commons() {
    let quiet = traces(&tree("normalDay"), &gate("normalDay"));
    assert!(!quiet.flows());
    assert_eq!(library_kind(&quiet), NodeKind::Library { active: false });

    let armed = tree("armedFolder");
    let open = gate("armedFolder");
    assert!(
        open.open(),
        "the recorded armedFolder day has nothing stopping a flow"
    );
    let automatic = armed.tools.iter().filter(|t| {
        t.mode != SourceMode::Off
            && t.folders
                .iter()
                .any(|f| f.mode == Some(ProjectMode::AutoUpload))
    });
    assert!(
        automatic.count() > 0,
        "the recorded armedFolder day arms a folder under a tool"
    );
    let scene = traces(&armed, &open);
    let moving = Scene::directories(&armed)
        .iter()
        .take(FOLDERS_SHOWN)
        .filter(|folder| {
            folder.mode == Some(ProjectMode::AutoUpload)
                && Scene::carriers(folder, &armed)
                    .iter()
                    .any(|t| t.mode != SourceMode::Off && open.sends_to_commons(t.kind))
        })
        .count();
    assert!(moving > 0);
    assert_eq!(
        flowing(&scene),
        moving + 1,
        "each moving folder's line, and the line out"
    );
    assert!(scene.flows());
    assert_eq!(library_kind(&scene), NodeKind::Library { active: true });
    // The library's card: what was contributed, a dash while History is
    // unread, never none.
    let words = words();
    assert_eq!(
        node(&scene, "library").unwrap().detail,
        library_words(&words, None)
    );
    assert!(library_words(&words, None).starts_with("\u{2014} "));
}

/// The node cards say #1146's sentences: this computer and the library
/// count what was contributed from History, a tool's title counts its
/// folders, and a folder says its path, rule and counts.
#[test]
fn the_node_cards_say_the_cores_sentences() {
    let words = words();
    let tree = tree("normalDay");
    let history: Vec<HistoryRecord> =
        serde_json::from_value(sample("normalDay", "list_history")["history"].clone()).unwrap();
    let contributed = Contributions::from_history(Some(&history));
    let scene = Scene::traces(
        &tree,
        &gate("normalDay"),
        &contributed,
        None,
        &BTreeMap::new(),
        SIZE,
    );
    let total = contributed.total.expect("the recorded page is whole");
    let waiting = tree.tools.iter().map(ToolNode::waiting).sum::<usize>()
        + tree
            .unplaced
            .iter()
            .map(|f| f.sessions.len())
            .sum::<usize>();
    assert_eq!(
        node(&scene, "hub").unwrap().detail,
        hub_words(&words, waiting, Some(total))
    );
    assert_eq!(
        node(&scene, "library").unwrap().detail,
        library_words(&words, Some(total))
    );
    for tool in &tree.tools {
        let drawn = node(&scene, &format!("tool:{}", tool.id())).unwrap();
        assert_eq!(drawn.label, tool.kind.display_name());
        assert_eq!(drawn.card_title(), tool_title(&words, tool));
        assert!(!drawn.detail.contains('{'), "{}", drawn.detail);
    }
    for folder in scene.nodes.iter().filter(|n| n.id.starts_with("folder:")) {
        assert!(folder.detail.contains(" waiting, "), "{}", folder.detail);
        assert!(!folder.detail.contains('{'), "{}", folder.detail);
    }
    // Unset is never said as off; unknown is said as neither.
    assert_eq!(
        tool_words(&words, SourceMode::Unknown, 0),
        words.tool_nothing_waiting
    );
    assert_ne!(tool_words(&words, SourceMode::Unset, 0), words.tool_off);
    assert_eq!(
        folder_words(&words, Some("/a"), Some("Ask"), 1, Some(2)),
        "/a. Rule: Ask. 1 trace waiting, 2 contributed."
    );
    assert_eq!(
        folder_words(&words, None, None, 3, None),
        "Rule: not set. 3 traces waiting, \u{2014} contributed."
    );
}

/// The cards' contributed counts are a whole count or a dash: a page that
/// came back as full as `list_history` was asked for is not a total, and a
/// failed read (`None`) is not current.
#[test]
fn a_capped_or_failed_history_counts_nothing_contributed() {
    let words = words();
    let first: serde_json::Value = sample("normalDay", "list_history")["history"][0].clone();
    let rows = |count: usize| -> Vec<HistoryRecord> {
        (0..count)
            .map(|i| {
                let mut row = first.clone();
                row["status"] = "accepted".into();
                row["source"] = "claude-code".into();
                row["project_id"] = "p1".into();
                row["submission_id"] = format!("row-{i}").into();
                serde_json::from_value(row).unwrap()
            })
            .collect()
    };
    let whole = Contributions::from_history(Some(&rows(HISTORY_LIMIT - 1)));
    assert_eq!(whole.total, Some(HISTORY_LIMIT - 1));
    assert_eq!(whole.tool(SourceKind::ClaudeCode), Some(HISTORY_LIMIT - 1));
    assert_eq!(whole.folder("p1"), Some(HISTORY_LIMIT - 1));

    let capped = Contributions::from_history(Some(&rows(HISTORY_LIMIT)));
    assert_eq!(capped.total, None, "a capped page is counted as the whole");
    assert_eq!(capped.tool(SourceKind::ClaudeCode), None);
    assert_eq!(capped.folder("p1"), None);
    let scene = Scene::traces(
        &tree("normalDay"),
        &gate("normalDay"),
        &capped,
        None,
        &BTreeMap::new(),
        SIZE,
    );
    assert_eq!(
        node(&scene, "library").unwrap().detail,
        library_words(&words, None)
    );

    assert_eq!(Contributions::from_history(None).total, None);
    assert_eq!(Contributions::from_history(Some(&rows(3))).total, Some(3));
}

/// Nothing moves to the commons while the core says something stops it:
/// paused, held for review, a voided grant, signed out, or any of those
/// unread. Each still draws the armed tool's arc, dashed.
#[test]
fn nothing_flows_while_the_core_says_it_is_stopped() {
    let armed = tree("armedFolder");
    let stops: Vec<(&str, Box<dyn Fn(&mut serde_json::Value)>)> = vec![
        ("paused", Box::new(|s| s["paused"] = true.into())),
        (
            "held",
            Box::new(|s| {
                s["automatic_contribution_held"] =
                    serde_json::json!({"held_sessions": 2, "reasons": [], "projects": []})
            }),
        ),
        (
            "held unread",
            Box::new(|s| s["automatic_contribution_held"] = serde_json::Value::Null),
        ),
        (
            "grant voided",
            Box::new(|s| {
                s["grant_voids"] =
                    serde_json::json!([{"id": 1, "kind": "project", "project_id": "p"}])
            }),
        ),
        (
            "voids unread",
            Box::new(|s| s["grant_voids"] = serde_json::Value::Null),
        ),
        ("signed out", Box::new(|s| s["logged_in"] = false.into())),
        (
            "paused unread",
            Box::new(|s| {
                s.as_object_mut().unwrap().remove("paused");
            }),
        ),
        (
            "sign-in unread",
            Box::new(|s| {
                s.as_object_mut().unwrap().remove("logged_in");
            }),
        ),
    ];
    for (name, edit) in stops {
        let stopped = gate_with("armedFolder", ScreenState::Ready, |s| edit(s));
        let scene = traces(&armed, &stopped);
        assert!(!scene.flows(), "{name}");
        assert!(
            scene.arcs.iter().any(|a| a.style == ArcStyle::Dashed),
            "{name}: the armed arc is still drawn, dashed"
        );
        assert_eq!(
            library_kind(&scene),
            NodeKind::Library { active: false },
            "{name}"
        );
    }
}

/// A screen that is not current never draws a flow: core down, loading,
/// paused, unknown, or a status or route the core did not answer.
#[test]
fn nothing_flows_unless_the_screen_is_ready() {
    let armed = tree("armedFolder");
    for state in [
        ScreenState::CoreDown,
        ScreenState::Loading,
        ScreenState::Paused,
        ScreenState::Unknown,
    ] {
        let stale = gate_with("armedFolder", state, |_| {});
        assert!(!traces(&armed, &stale).flows(), "{state:?}");
    }
    let mut unread = gate("armedFolder");
    unread.status = None;
    assert!(!traces(&armed, &unread).flows(), "no status");
    let mut unread = gate("armedFolder");
    unread.destinations = None;
    assert!(!traces(&armed, &unread).flows(), "no tool_destinations");
    let mut unrouted = gate("armedFolder");
    unrouted.destinations.as_mut().unwrap().sessions_route = Some("not_enrolled".into());
    assert!(!traces(&armed, &unrouted).flows(), "no route for sessions");
    let nothing = traces(&armed, &CommonsGate::with_state(ScreenState::CoreDown));
    assert_eq!(
        node(&nothing, "library").unwrap().detail,
        library_words(&words(), None)
    );
}

/// The screen state's precedence: a failed read over everything, then
/// nothing read yet, then paused, then anything unsaid.
#[test]
fn the_screen_state_follows_the_macs_precedence() {
    assert_eq!(
        ScreenState::resolve(true, true, Some(false), true),
        ScreenState::Unknown
    );
    assert_eq!(
        ScreenState::resolve(false, false, Some(false), true),
        ScreenState::Loading
    );
    assert_eq!(
        ScreenState::resolve(false, true, Some(true), true),
        ScreenState::Paused
    );
    assert_eq!(
        ScreenState::resolve(false, true, None, true),
        ScreenState::Unknown
    );
    assert_eq!(
        ScreenState::resolve(false, true, Some(false), false),
        ScreenState::Unknown
    );
    assert_eq!(
        ScreenState::resolve(false, true, Some(false), true),
        ScreenState::Ready
    );
}

#[test]
fn dot_pair_spaces_the_count_and_dashes_an_unread_one() {
    assert_eq!(dot_pair("Sessions", Some(3)), "Sessions \u{b7} 3");
    assert_eq!(dot_pair("Sessions", None), "Sessions \u{b7} \u{2014}");
}

/// An off tool never flows, even with an armed folder under it.
#[test]
fn an_off_tool_never_flows() {
    let open = gate("armedFolder");
    let folder = FolderNode::new("f", "f", Some(ProjectMode::AutoUpload));
    let off = TracesTree {
        tools: vec![ToolNode::new(
            SourceKind::ClaudeCode,
            SourceMode::Off,
            vec![folder],
        )],
        ..TracesTree::default()
    };
    let scene = traces(&off, &open);
    assert!(!scene.flows());
    assert!(
        scene
            .arcs
            .iter()
            .all(|a| a.style != ArcStyle::Flowing && a.style != ArcStyle::Dashed)
    );
}

/// An off tool is crossed out; an unset one is never off.
#[test]
fn off_tools_are_crossed_out_and_unset_is_not() {
    let tree = tree("normalDay");
    let scene = traces(&tree, &gate("normalDay"));
    for tool in &tree.tools {
        let drawn = node(&scene, &format!("tool:{}", tool.id())).unwrap();
        let NodeKind::Tool { off, .. } = drawn.kind else {
            panic!("{} is not a tool", drawn.id)
        };
        assert_eq!(off, tool.mode == SourceMode::Off, "{}", tool.id());
    }
}

/// Folders past the most the column shows are counted, not drawn.
#[test]
fn extra_folders_are_counted() {
    let scene = traces(
        &one_tool(folders(13)),
        &CommonsGate::with_state(ScreenState::Loading),
    );
    assert_eq!(drawn_folders(&scene), FOLDERS_SHOWN);
    assert_eq!(
        scene.reveals.iter().map(|r| r.hidden).collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(
        scene
            .reveals
            .iter()
            .map(FolderReveal::next)
            .collect::<Vec<_>>(),
        [3]
    );
}

/// The "+N" pill draws the next six folders a click, until every one is
/// drawn; then it folds them back.
#[test]
fn the_folder_pill_reveals_six_more_then_folds_back() {
    let tree = one_tool(folders(20));
    let gate = CommonsGate::with_state(ScreenState::Ready);
    let mut revealed = BTreeMap::new();
    for (drawn, hidden) in [(10, 10), (16, 4), (20, 0)] {
        let scene = Scene::traces(
            &tree,
            &gate,
            &Contributions::unread(),
            None,
            &revealed,
            SIZE,
        );
        let reveal = scene.reveals.first().expect("a pill").clone();
        assert_eq!(drawn_folders(&scene), drawn, "{revealed:?}");
        assert_eq!(reveal.hidden, hidden, "{revealed:?}");
        assert_eq!(drawn_folders(&scene) + reveal.hidden, 20);
        // Every folder drawn sits in the band, however many there are.
        let band = Scene::band(scene.space);
        for node in scene.nodes.iter().filter(|n| n.id.starts_with("folder:")) {
            assert!(
                (band.0..=band.1).contains(&node.at.y),
                "{} off the band with {revealed:?}",
                node.id
            );
        }
        assert!(
            (band.0..=band.1).contains(&reveal.at.y),
            "the pill is off the band"
        );
        revealed = Scene::revealing(&reveal, &revealed);
    }
    assert!(
        revealed.is_empty(),
        "the fold-back pill returns the column to the first ten"
    );
    let folded = Scene::traces(
        &tree,
        &gate,
        &Contributions::unread(),
        None,
        &revealed,
        SIZE,
    );
    assert_eq!(drawn_folders(&folded), FOLDERS_SHOWN);
    assert_eq!(folded.space, SIZE);
}

/// Folders a pill drew past what the band holds zoom the map out: the
/// design space grows, its shape kept, and the line height holds.
#[test]
fn revealed_folders_past_the_band_zoom_the_map_out() {
    let revealed = BTreeMap::from([(FOLDER_COLUMN.to_string(), 30)]);
    let scene = Scene::traces(
        &one_tool(folders(40)),
        &CommonsGate::with_state(ScreenState::Ready),
        &Contributions::unread(),
        None,
        &revealed,
        SIZE,
    );
    assert!(scene.space.height > SIZE.height);
    assert!((scene.space.width / scene.space.height - SIZE.width / SIZE.height).abs() < 0.0001);
    let mut ys: Vec<f64> = scene
        .nodes
        .iter()
        .filter(|n| n.id.starts_with("folder:"))
        .map(|n| n.at.y)
        .collect();
    ys.sort_by(f64::total_cmp);
    assert_eq!(ys.len(), 40);
    let tightest = ys
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min);
    assert!(tightest >= FOLDER_STEP_MIN - 0.5);
}

/// The pill's accessible names say how many it draws.
#[test]
fn the_folder_pill_is_named() {
    let words = words();
    assert_eq!(reveal_folders(&words, 6), "Show 6 more folders");
    assert_eq!(reveal_folders(&words, 1), "Show 1 more folder");
    assert_eq!(words.fold_folders, "Show fewer");
    assert_eq!(words.fold_folders_label, "Show fewer folders");
}

/// The library is lit once anything was contributed, armed or not, and no
/// line reaches it: a contribution flies there from the pad.
#[test]
fn the_library_is_lit_by_what_was_contributed() {
    let tree = one_tool(vec![FolderNode::new("f", "f", Some(ProjectMode::Ask))]);
    let gate = CommonsGate::with_state(ScreenState::Ready);
    let scene = |contributed: &Contributions| {
        Scene::traces(&tree, &gate, contributed, None, &BTreeMap::new(), SIZE)
    };
    let none = scene(&Contributions::counted(0, &[]));
    assert_eq!(library_kind(&none), NodeKind::Library { active: false });
    let sent = scene(&Contributions::counted(4, &[("claude-code", 4)]));
    assert_eq!(library_kind(&sent), NodeKind::Library { active: true });
    let library = node(&sent, "library").unwrap().at;
    assert!(
        sent.arcs.iter().all(|a| a.to != library),
        "a line reaches the library"
    );
    assert!(!sent.flows(), "a past contribution never moves");
    let unread = scene(&Contributions::unread());
    assert_eq!(library_kind(&unread), NodeKind::Library { active: false });
}

/// A selection's tool is ringed, the other tools and the folders it has no
/// traces in faded to 0.3; nothing selected fades nothing.
#[test]
fn the_selections_tool_is_ringed_and_the_others_fade() {
    let tree = TracesTree {
        tools: vec![
            ToolNode::new(
                SourceKind::ClaudeCode,
                SourceMode::Watch,
                vec![FolderNode::new("f", "f", Some(ProjectMode::Ask))],
            ),
            ToolNode::new(SourceKind::Codex, SourceMode::Watch, vec![]),
        ],
        ..TracesTree::default()
    };
    let gate = CommonsGate::with_state(ScreenState::Ready);
    let contributed = Contributions::counted(2, &[("claude-code", 1), ("codex", 1)]);
    let plain = Scene::traces(&tree, &gate, &contributed, None, &BTreeMap::new(), SIZE);
    assert!(plain.nodes.iter().all(|n| !n.ringed));
    assert!(
        plain
            .nodes
            .iter()
            .filter(|n| n.id.starts_with("tool:") && !n.dashed)
            .all(|n| n.dim == 1.0)
    );
    let scene = Scene::traces(
        &tree,
        &gate,
        &contributed,
        Some("codex"),
        &BTreeMap::new(),
        SIZE,
    );
    let codex = node(&scene, "tool:codex").unwrap();
    let claude = node(&scene, "tool:claude-code").unwrap();
    assert!(codex.ringed);
    assert_eq!(codex.dim, 1.0);
    assert!(!claude.ringed);
    assert_eq!(claude.dim, 0.3);
    // The folder only Claude Code carries fades with it, line and all.
    assert_eq!(node(&scene, "folder:f").unwrap().dim, 0.3);
    let port = scene.stack.as_ref().unwrap().port_left();
    assert_eq!(
        scene
            .arcs
            .iter()
            .filter(|a| a.to == port)
            .map(|a| a.dim)
            .collect::<Vec<_>>(),
        [0.3]
    );
    // Every tool stays drawn: the binoculars move the camera, they do not
    // filter the map.
    assert_eq!(scene.focus_point(Some("codex")), Some(codex.at));
    assert_eq!(scene.focus_point(None), None);
}

/// The binoculars' camera: the tool's point lands at 1.6x on the same
/// place #1146's transform puts it.
#[test]
fn the_focus_camera_scales_about_the_tool() {
    let focus = Point::new(150.0, 170.0);
    let fit = MapGeometry::new(SIZE, 1.0, Some(focus), SIZE);
    assert!((fit.scale - 1.6).abs() < 0.0001);
    let drawn = fit.point(focus);
    assert!((drawn.x - (290.0 + 1.6 * 50.0)).abs() < 0.001);
    assert!((drawn.y - 380.0).abs() < 0.001);
    assert!((MapGeometry::new(SIZE, 1.0, None, SIZE).scale - 1.0).abs() < 0.0001);
}

#[test]
fn the_design_space_fits_and_centres() {
    let size = Size::new(600.0, 700.0);
    let fit = MapGeometry::new(size, 1.0, None, SIZE);
    let top_left = fit.point(Point::new(0.0, 0.0));
    let bottom_right = fit.point(Point::new(SIZE.width, SIZE.height));
    assert!(top_left.x >= -0.001 && top_left.y >= -0.001);
    assert!(bottom_right.x <= size.width + 0.001 && bottom_right.y <= size.height + 0.001);
    let centre = Point::new(SIZE.width / 2.0, SIZE.height / 2.0);
    for zoom in [0.6, 1.0, 2.0] {
        let p = MapGeometry::new(size, zoom, None, SIZE).point(centre);
        assert!((p.x - size.width / 2.0).abs() < 0.001);
        assert!((p.y - size.height / 2.0).abs() < 0.001);
    }
}

/// The design space takes the pane's shape at #1146's area, held between
/// tall and wide limits.
#[test]
fn the_design_space_takes_the_panes_shape() {
    assert_eq!(Scene::space_for(Size::new(0.0, 100.0)), SIZE);
    let wide = Scene::space_for(Size::new(1200.0, 500.0));
    assert!((wide.width * wide.height - SIZE.width * SIZE.height).abs() < 0.01);
    assert!((wide.width / wide.height - 1.7).abs() < 0.0001);
    let tall = Scene::space_for(Size::new(100.0, 1000.0));
    assert!((tall.width / tall.height - 0.42).abs() < 0.0001);
}

#[test]
fn zoom_steps_by_a_quarter_between_its_limits() {
    let mut zoom = 1.0;
    for _ in 0..10 {
        zoom = zoomed(zoom, true);
    }
    assert_eq!(zoom, ZOOM_MAX);
    for _ in 0..10 {
        zoom = zoomed(zoom, false);
    }
    assert_eq!(zoom, ZOOM_MIN);
    assert_eq!(zoomed(1.0, true), 1.25);
}

/// A node's halo scales with the map, as its disc does.
#[test]
fn the_ring_scales_with_zoom() {
    let base = ring_metrics(MapGeometry::new(SIZE, 1.0, None, SIZE).scale);
    assert!((base.0 - 4.0).abs() < 0.0001);
    assert!((base.1 - 3.0).abs() < 0.0001);
    for zoom in [ZOOM_MIN, 1.5, ZOOM_MAX] {
        let ring = ring_metrics(MapGeometry::new(SIZE, zoom, None, SIZE).scale);
        assert!(
            (ring.0 - 4.0 * zoom).abs() < 0.0001,
            "offset at zoom {zoom}"
        );
        assert!((ring.1 - 3.0 * zoom).abs() < 0.0001, "width at zoom {zoom}");
    }
}

/// The library's and this computer's cards sit beside the station and the
/// pad, never over them: on the right in a field wide enough, else on the
/// left, and always inside the field.
#[test]
fn the_library_and_this_computer_cards_sit_beside_their_drawings() {
    let card = NODE_CARD_WIDTH;
    let library = Node::new(
        "library",
        NodeKind::Library { active: true },
        Point::default(),
        22.0,
        "",
        "",
    );
    let hub = Node::new("hub", NodeKind::Hub, Point::default(), 16.0, "", "");
    // `FlowMapView.drawnHalfWidth`: the station 150 wide, the orb 104,
    // the pad or the laptop the pad's half-width either way.
    assert_eq!(drawn_half_width(&library, true, MapStyle::Launch), 75.0);
    assert_eq!(drawn_half_width(&library, true, MapStyle::Standard), 52.0);
    assert_eq!(
        drawn_half_width(&hub, true, MapStyle::Launch),
        PAD_HALF_WIDTH
    );
    assert_eq!(
        drawn_half_width(&hub, true, MapStyle::Standard),
        PAD_HALF_WIDTH
    );
    assert_eq!(drawn_half_width(&library, false, MapStyle::Launch), 22.0);

    for (pane, side) in [
        (Size::new(600.0, 800.0), "left"),
        (Size::new(1500.0, 800.0), "right"),
    ] {
        let space = Scene::space_for(pane);
        let fit = MapGeometry::new(pane, 1.0, None, space);
        for (node, point) in [(&library, Scene::library(space)), (&hub, Scene::hub(space))] {
            let centre = fit.point(point).x;
            let half = drawn_half_width(node, true, MapStyle::Launch) * fit.scale;
            let x = card_x(centre, half, card, pane.width);
            let (lo, hi) = (x - card / 2.0, x + card / 2.0);
            let what = format!("{}, {} wide", node.id, pane.width);
            assert!(hi <= centre - half || lo >= centre + half, "{what}");
            assert!(lo >= CARD_MARGIN, "{what}");
            assert!(hi <= pane.width - CARD_MARGIN, "{what}");
            assert_eq!(if x > centre { "right" } else { "left" }, side, "{what}");
        }
    }
    // Neither side holds it whole: the roomier side, inside the field.
    let x = card_x(320.0, 60.0, card, 440.0);
    assert!(x < 320.0);
    assert_eq!(x - card / 2.0, CARD_MARGIN);
}

/// A node's label and sublabel never meet: 11 and 10 point type 14 apart,
/// scaled with the map, never below the floor.
#[test]
fn node_labels_never_touch() {
    for scale in [0.3, 0.6, 0.85, 1.0, 1.4, 2.0] {
        let (label, sublabel, label_offset, sublabel_offset) = label_type(scale);
        assert!(sublabel < label, "the sublabel is the smaller caption");
        assert!(
            sublabel_offset - label_offset > 0.6 * label + 0.6 * sublabel,
            "at {scale}"
        );
        assert!(label >= 11.0 * LABEL_FLOOR);
    }
    assert_eq!(label_type(1.0).0, 11.0);
    assert_eq!(label_type(1.0).1, 10.0);
    // The credential's key is #1146's level outline, wider than tall.
    let ((bow, radius), segments) = key_glyph(Point::default(), 1.0);
    let xs = segments
        .iter()
        .flat_map(|(a, b)| [a.x, b.x])
        .chain([bow.x - radius]);
    let ys = segments
        .iter()
        .flat_map(|(a, b)| [a.y, b.y])
        .chain([bow.y - radius, bow.y + radius]);
    let width = xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min);
    let height = ys.clone().fold(f64::MIN, f64::max) - ys.fold(f64::MAX, f64::min);
    assert!(width > height);
}

/// A folder's name is cut at fourteen characters, the last an ellipsis;
/// the full name stays its accessible name.
#[test]
fn long_folder_names_are_cut_with_an_ellipsis() {
    assert_eq!(short_label("infra"), "infra");
    assert_eq!(short_label("abcdefghijklmn"), "abcdefghijklmn");
    assert_eq!(short_label("abcdefghijklmno"), "abcdefghijklm\u{2026}");
    assert_eq!(short_label("abcdefghijklmno").chars().count(), LABEL_LIMIT);
    assert_eq!(folder_id("folder:p1"), Some("p1"));
    assert_eq!(folder_id("tool:codex"), None);
}

/// Other devices hang below this computer with no line to the commons;
/// past four the rest are counted, and one with contributions lights the
/// library.
#[test]
fn other_devices_hang_below_this_computer() {
    let base = traces(
        &tree("normalDay"),
        &CommonsGate::with_state(ScreenState::Ready),
    );
    assert_eq!(base.adding_devices(&[], false), base);
    let devices: Vec<OtherDevice> = (0..6)
        .map(|i| OtherDevice {
            id: format!("d{i}"),
            name: format!("d{i}"),
            detail: "\u{2014}".into(),
            contributed: (i == 5).then_some(2),
        })
        .collect();
    let scene = base.adding_devices(&devices, true);
    let hub = node(&scene, "hub").unwrap().at;
    let drawn: Vec<&Node> = scene
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Device)
        .collect();
    assert_eq!(drawn.len(), DEVICES_SHOWN);
    assert!(drawn.iter().all(|n| n.at.x == hub.x && n.at.y > hub.y));
    assert_eq!(
        scene.overflows.iter().map(|o| o.count).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(scene.arcs, base.arcs, "no line joins another device");
    assert_eq!(library_kind(&scene), NodeKind::Library { active: true });
    let band = Scene::band(scene.space);
    assert!(scene.add_device.is_some_and(|p| p.y <= band.1));
}

/// The pile's card opens under the pointer: taller, then wider.
#[test]
fn the_tool_pile_opens_taller_then_wider() {
    let scene = traces(
        &tree("normalDay"),
        &CommonsGate::with_state(ScreenState::Ready),
    );
    let stack = scene.stack.unwrap();
    let fit = MapGeometry::new(SIZE, 1.0, None, SIZE);
    let closed = stack.card(&fit, false, false);
    let tall = stack.card(&fit, true, false);
    let wide = stack.card(&fit, true, true);
    assert!(tall.3 > closed.3);
    assert_eq!(tall.2, closed.2);
    assert!(wide.2 > tall.2);
    assert_eq!(closed.0, fit.point(stack.port_left()).x);
    let first = stack.coin_center(0, true);
    let last = stack.coin_center(stack.tools.len() - 1, true);
    assert!(
        (last.y - first.y - (stack.tools.len() - 1) as f64 * ToolStack::OPEN_STEP).abs() < 0.001
    );
    assert!(((first.y + last.y) / 2.0 - stack.center.y).abs() < 0.001);
}

/// A tool is drawn as two letters of its name until the look slice brings
/// its artwork.
#[test]
fn a_tool_without_artwork_is_two_letters_of_its_name() {
    assert_eq!(initials("Cursor"), "Cu");
    assert_eq!(initials("GitHub Copilot CLI"), "GC");
    assert_eq!(initials("Claude Code"), "CC");
    assert_eq!(initials("Codex"), "Co");
    assert_eq!(initials(""), "?");
}

fn pending(id: &str, project: &str, source: &str) -> QueueEntry {
    QueueEntry {
        entry_id: id.into(),
        project_id: project.into(),
        // Every folder named alike: identity is the id, never the label.
        project_label: "same label".into(),
        source: source.into(),
        state: "pending".into(),
        ..QueueEntry::default()
    }
}

fn folder_ids(scene: &Scene) -> Vec<String> {
    let mut ids: Vec<String> = scene
        .nodes
        .iter()
        .filter(|n| n.id.starts_with("folder:"))
        .map(|n| n.id.clone())
        .collect();
    ids.sort();
    ids
}

/// The map's folders are the queue's folders by id: two folders with the
/// same label are two nodes, each waiting session is counted once, and an
/// entry that is not pending draws nothing (carried over from the old
/// map's `pending_relationships_use_ids_and_count_each_entry_once`).
#[test]
fn folders_are_drawn_by_id_and_count_each_pending_entry_once() {
    let entries = vec![
        pending("a", "p1", "codex"),
        pending("b", "p1", "claude-code"),
        pending("c", "p2", "codex"),
        QueueEntry {
            state: "approved".into(),
            ..pending("d", "p3", "codex")
        },
    ];
    let tree = TracesTree::build(&entries, &[], None);
    let scene = traces(&tree, &CommonsGate::with_state(ScreenState::Ready));
    assert_eq!(folder_ids(&scene), ["folder:p1", "folder:p2"]);
    let waiting = tree.tools.iter().map(ToolNode::waiting).sum::<usize>()
        + tree
            .unplaced
            .iter()
            .map(|f| f.sessions.len())
            .sum::<usize>();
    assert_eq!(waiting, 3);
    let p1 = node(&scene, "folder:p1").unwrap();
    assert!(p1.detail.contains("2 traces waiting"), "{}", p1.detail);
    // Both tools carry p1, so selecting either keeps it lit.
    let carriers: Vec<_> = Scene::carriers(&tree.folders[0], &tree)
        .iter()
        .map(|t| t.id())
        .collect();
    assert!(
        carriers.contains(&"codex") && carriers.contains(&"claude-code"),
        "{carriers:?}"
    );
}

/// The same entries in another order draw the same nodes, and a folder
/// whose last entry leaves is no longer drawn (carried over from the old
/// map's `selection_follows_identity_after_reordering_and_removal`).
#[test]
fn nodes_follow_identity_after_reordering_and_removal() {
    let mut entries = vec![pending("a", "p1", "codex"), pending("b", "p2", "codex")];
    let gate = CommonsGate::with_state(ScreenState::Ready);
    let before = folder_ids(&traces(&TracesTree::build(&entries, &[], None), &gate));
    entries.reverse();
    let after = folder_ids(&traces(&TracesTree::build(&entries, &[], None), &gate));
    assert_eq!(before, after);
    entries.retain(|e| e.entry_id != "a");
    let removed = traces(&TracesTree::build(&entries, &[], None), &gate);
    assert_eq!(folder_ids(&removed), ["folder:p2"]);
    // Nothing pending still draws the map: this computer and the library.
    let empty = traces(&TracesTree::build(&[], &[], None), &gate);
    assert!(node(&empty, "hub").is_some() && node(&empty, "library").is_some());
}

// --- The Private AI map ------------------------------------------------------

fn harness_rows(set: &str) -> Vec<HarnessRow> {
    let list: crate::model::HarnessList =
        serde_json::from_value(sample(set, "harness_list")).unwrap();
    list.harnesses.iter().map(HarnessRow::from_model).collect()
}

/// `FlowMapLoadTests.harnesses`: `count` installed tools with no artwork.
fn many_harnesses(count: usize) -> Vec<HarnessRow> {
    (0..count)
        .map(|i| HarnessRow {
            id: format!("h{i}"),
            name: format!("Agent {i}"),
            installed: true,
            state: "connected".into(),
        })
        .collect()
}

fn private_ai(
    rows: Option<&[HarnessRow]>,
    private_ai: Option<&str>,
    state: HarnessState,
    destinations: Option<&ToolDestinations>,
    space: Size,
) -> Scene {
    Scene::private_ai(rows, private_ai, |_| None, |_| state, destinations, space)
}

fn managed(wire: serde_json::Value) -> ToolDestinations {
    serde_json::from_value(wire).unwrap()
}

/// The least gap between neighbours in a column of nodes.
fn tightest(nodes: &[&Node]) -> f64 {
    let mut ys: Vec<f64> = nodes.iter().map(|n| n.at.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min)
}

/// `test_managedRoutesAreSeparateAndNeverAnimateUnobservedCalls`.
#[test]
fn managed_routes_are_separate_and_never_animate_unobserved_calls() {
    let destinations = managed(serde_json::json!({
        "tools": [],
        "managed_copy": {"opencode": "OpenCode", "provider_openai": "OpenAI",
            "provider_near_ai": "NEAR AI", "state_running": "Running",
            "collection_off": "Trace collection off",
            "collection_pending": "Trace collection pending", "title": "Managed sessions"},
        "managed_sessions": [
            {"id": "a", "tool": "opencode", "connection": "api_key", "provider": "openai",
             "model": "gpt-4.1", "state": "running", "collection_state": "off"},
            {"id": "b", "tool": "opencode", "connection": "near_ai", "provider": "near_ai",
             "model": "model-b", "state": "running", "collection_state": "pending"}
        ]
    }));
    let scene = private_ai(None, None, HarnessState::Unknown, None, SIZE)
        .adding_managed_routes(Some(&destinations));
    assert_eq!(scene.nodes.len(), 4);
    let ids: std::collections::BTreeSet<_> = scene.nodes.iter().map(|n| n.id.clone()).collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(
        scene.arcs.iter().map(|a| a.style).collect::<Vec<_>>(),
        [ArcStyle::Dashed, ArcStyle::Dashed]
    );
    assert!(!scene.flows());
    assert!(
        node(&scene, "destination").is_none(),
        "managed routes never imply a global harness connection"
    );
    assert!(
        scene
            .nodes
            .iter()
            .any(|n| n.detail.contains("Trace collection off"))
    );
    assert!(scene.nodes.iter().any(|n| n.label == "OpenAI"));
    assert!(scene.nodes.iter().any(|n| n.label == "NEAR AI"));
    assert!(scene.nodes.iter().all(|n| SIZE.contains(n.at)));
    // A managed route's tool card: its name, the copy's title as its card
    // title, and a count of its sessions under it.
    let tool = node(&scene, "managed:opencode:openai").unwrap();
    assert_eq!(tool.label, "OpenCode");
    assert_eq!(tool.title.as_deref(), Some("Managed sessions"));
    assert_eq!(tool.sublabel.as_deref(), Some("Managed sessions \u{b7} 1"));
    assert!(matches!(
        tool.kind,
        NodeKind::Harness {
            tool: Some(SourceKind::Opencode),
            installed: true
        }
    ));
    let provider = node(&scene, "managed-provider:opencode:near_ai").unwrap();
    assert_eq!(provider.kind, NodeKind::Destination(Lamp::Unknown));
    assert_eq!(provider.radius, 16.0);
    assert!(
        Scene::default()
            .adding_managed_routes(None)
            .nodes
            .is_empty()
    );
}

/// `test_managedRoutesStayClearOfTheDestination`: right of and below the
/// destination in any design space, so their names never cover its card.
#[test]
fn managed_routes_stay_clear_of_the_destination() {
    let destinations = managed(serde_json::json!({
        "tools": [],
        "managed_copy": {"title": "Managed sessions"},
        "managed_sessions": [
            {"id": "a", "tool": "opencode", "connection": "api_key", "provider": "openai",
             "model": "m", "state": "running", "collection_state": "off"},
            {"id": "b", "tool": "opencode", "connection": "near_ai", "provider": "near_ai",
             "model": "m", "state": "running", "collection_state": "off"},
            {"id": "c", "tool": "claude", "connection": "api_key", "provider": "anthropic",
             "model": null, "state": "running", "collection_state": "off"}
        ]
    }));
    for space in [
        SIZE,
        Scene::space_for(Size::new(1200.0, 500.0)),
        Scene::space_for(Size::new(400.0, 1000.0)),
    ] {
        let scene = private_ai(None, None, HarnessState::Unknown, None, space)
            .adding_managed_routes(Some(&destinations));
        let destination = Scene::destination(space);
        assert_eq!(scene.nodes.len(), 6);
        for n in &scene.nodes {
            assert!(space.contains(n.at), "{} outside {space:?}", n.id);
            assert!(n.at.x > destination.x, "{}", n.id);
            assert!(
                n.at.y - destination.y > 60.0,
                "{} sits on the destination in {space:?}",
                n.id
            );
        }
        // An unknown provider is said as unknown, never left blank.
        let anthropic = node(&scene, "managed-provider:claude:anthropic").unwrap();
        assert_eq!(anthropic.label, monitor_screens_copy().unknown);
    }
}

/// `test_openCodeOnlyInstallHidesAbsentGlobalHarnessChoices`.
#[test]
fn an_opencode_only_install_hides_absent_global_harness_choices() {
    let rows = harness_rows("normalDay");
    let destinations = managed(serde_json::json!({"tools": [
        {"tool": "claude-code", "installed": false},
        {"tool": "codex", "installed": false},
        {"tool": "opencode", "installed": true}
    ]}));
    let scene = private_ai(
        Some(&rows),
        Some("off"),
        HarnessState::NotConnected,
        Some(&destinations),
        SIZE,
    );
    assert!(node(&scene, "harness:claude").is_none());
    assert!(node(&scene, "harness:codex").is_none());
    assert!(node(&scene, "harness:opencode").is_some());
    assert!(!scene.flows());
}

/// `test_thePrivateAISceneFollowsTheCoresVerdict`: each tool's arc follows
/// the core's state for it, never `connected`; every state has its own
/// look, and only answering moves.
#[test]
fn the_private_ai_scene_follows_the_cores_verdict() {
    let rows = harness_rows("normalDay");
    let words = words();
    let expected = [
        (HarnessState::Answering, ArcStyle::Flowing),
        (HarnessState::ConnectedNoCalls, ArcStyle::Quiet),
        (HarnessState::ActivityShared, ArcStyle::Shared),
        (HarnessState::NotConnected, ArcStyle::Dashed),
        (HarnessState::Unknown, ArcStyle::Unknown),
    ];
    let styles: std::collections::BTreeSet<_> =
        expected.iter().map(|(_, s)| format!("{s:?}")).collect();
    assert_eq!(
        styles.len(),
        expected.len(),
        "every state draws differently"
    );
    for (state, style) in expected {
        let scene = private_ai(Some(&rows), Some("running"), state, None, SIZE);
        assert_eq!(scene.nodes.len(), rows.len() + 1);
        // #1146: the node is the credential, with its tools under it.
        let first = &scene.nodes[0];
        assert_eq!(first.label, words.credential);
        let connected = if matches!(state, HarnessState::NotConnected | HarnessState::Unknown) {
            0
        } else {
            rows.len()
        };
        assert_eq!(
            first.sublabel.as_deref(),
            Some(connected_words(&words, connected, false).as_str())
        );
        assert!(scene.arcs.iter().all(|a| a.style == style), "{state:?}");
        assert_eq!(scene.flows(), state == HarnessState::Answering, "{state:?}");
        let lamp = if state == HarnessState::Answering {
            Lamp::Answering
        } else {
            Lamp::Running
        };
        assert_eq!(first.kind, NodeKind::Destination(lamp), "{state:?}");
    }
    // No sentence from the core: a dash, never words of the shell's own.
    let scene = private_ai(Some(&rows), None, HarnessState::Unknown, None, SIZE);
    assert!(scene.nodes[1..].iter().all(|n| n.detail == "\u{2014}"));
}

/// The credential's card: how many tools are connected, in a sentence,
/// and unknown said after it, never left to read as off.
#[test]
fn the_credentials_card_says_how_many_are_connected_and_unknown() {
    let rows = harness_rows("normalDay");
    let words = words();
    let scene = private_ai(
        Some(&rows),
        Some("running"),
        HarnessState::Answering,
        None,
        SIZE,
    );
    assert_eq!(
        scene.nodes[0].detail,
        connected_words(&words, rows.len(), true)
    );
    assert_eq!(
        connected_words(&words, 1, true),
        words.connected.replace("{label}", words.tools_one)
    );
    assert_eq!(
        connected_words(&words, 3, false),
        words
            .connected_line
            .replace("{label}", &words.tools.replace("{count}", "3"))
    );
    let unknown = private_ai(Some(&rows), None, HarnessState::Answering, None, SIZE);
    assert_eq!(unknown.nodes[0].kind, NodeKind::Destination(Lamp::Unknown));
    assert_eq!(
        unknown.nodes[0].detail,
        format!(
            "{} {}",
            connected_words(&words, rows.len(), true),
            monitor_screens_copy().unknown
        )
    );
}

/// `test_theDestinationPaintsFromThePrivateAIState`: unknown is kept apart
/// from off.
#[test]
fn the_destination_paints_from_the_private_ai_state() {
    assert_eq!(Scene::lamp(Some("running"), true), Lamp::Answering);
    assert_eq!(Scene::lamp(Some("running"), false), Lamp::Running);
    assert_eq!(Scene::lamp(Some("off"), false), Lamp::Off);
    assert_eq!(Scene::lamp(None, false), Lamp::Unknown);
    assert_eq!(
        Scene::lamp(Some("a-state-from-a-later-daemon"), false),
        Lamp::Unknown
    );
    // Calls are never answered by a destination the core says is off.
    assert_eq!(Scene::lamp(Some("off"), true), Lamp::Off);
}

/// The state label the credential paints from: `tool_destinations`'s,
/// then the status's, as `PrivateAIFlowMap` reads them.
#[test]
fn the_private_ai_label_is_the_destinations_then_the_statuss() {
    let status: GateStatus = serde_json::from_value(sample("normalDay", "status")).unwrap();
    let destinations = destinations("normalDay");
    assert_eq!(destinations.private_ai.as_deref(), Some("off"));
    assert_eq!(
        private_ai_label(Some(&destinations), Some(&status)).as_deref(),
        Some("off")
    );
    let silent = managed(serde_json::json!({"tools": []}));
    assert_eq!(
        private_ai_label(Some(&silent), Some(&status)).as_deref(),
        Some("running")
    );
    assert_eq!(
        private_ai_label(None, Some(&status)).as_deref(),
        Some("running")
    );
    assert_eq!(private_ai_label(None, None), None);
}

/// `test_nothingAnsweringIsDrawnStill`.
#[test]
fn nothing_answering_is_drawn_still() {
    let rows = harness_rows("empty");
    let scene = private_ai(
        Some(&rows),
        Some("off"),
        HarnessState::NotConnected,
        None,
        SIZE,
    );
    assert!(!scene.flows());
    assert_eq!(scene.nodes[0].kind, NodeKind::Destination(Lamp::Off));
}

/// `test_aMissingToolNeverSaysItIsNotConnected` and `rowSentence`: a tool
/// that is not here says so and nothing else; a state with no sentence
/// says nothing, never a borrowed one.
#[test]
fn a_missing_tool_never_says_it_is_not_connected() {
    let row = |installed: bool, state: &str| HarnessRow {
        id: "gemini".into(),
        name: "Gemini CLI".into(),
        installed,
        state: state.into(),
    };
    assert_eq!(
        row_sentence(&row(false, "not_connected")).as_deref(),
        Some(crate::copy::HARNESS_NOT_INSTALLED)
    );
    assert_eq!(
        row_sentence(&row(true, "answering")).as_deref(),
        Some(crate::copy::harness_state_line("answering"))
    );
    // The core's not-connected sentence is empty: a dash on the card.
    assert_eq!(crate::copy::harness_state_line("not_connected"), "");
    assert_eq!(row_sentence(&row(true, "not_connected")), None);
    assert_eq!(row_sentence(&row(true, "activity_shared")), None);
    assert_eq!(row_sentence(&row(true, "unknown")), None);
    assert_eq!(harness_state("answering"), HarnessState::Answering);
    assert_eq!(
        harness_state("a-state-from-a-later-daemon"),
        HarnessState::Unknown
    );
}

/// `test_harnessIdsMapToTheToolsWithArtwork` (`HarnessToolArt`).
#[test]
fn harness_ids_map_to_the_tools_with_artwork() {
    assert_eq!(harness_tool("claude"), Some(SourceKind::ClaudeCode));
    assert_eq!(harness_tool("claude-code"), Some(SourceKind::ClaudeCode));
    assert_eq!(harness_tool("codex"), Some(SourceKind::Codex));
    assert_eq!(harness_tool("gemini"), Some(SourceKind::GeminiCli));
    assert_eq!(harness_tool("gemini-cli"), Some(SourceKind::GeminiCli));
    assert_eq!(harness_tool("cline"), Some(SourceKind::Cline));
    assert_eq!(harness_tool("opencode"), Some(SourceKind::Opencode));
    assert_eq!(harness_tool("something-new"), None);
}

/// `test_aLoadedPrivateAIMapFitsItsBandAndCountsEveryTool`.
#[test]
fn a_loaded_private_ai_map_fits_its_band_and_counts_every_tool() {
    let words = words();
    for count in [6, 12, 20, 40] {
        let rows = many_harnesses(count);
        for pane in [
            Size::new(440.0, 860.0),
            Size::new(900.0, 560.0),
            Size::new(360.0, 560.0),
        ] {
            let space = Scene::space_for(pane);
            let band = Scene::band(space);
            let scene = private_ai(
                Some(&rows),
                Some("running"),
                HarnessState::ConnectedNoCalls,
                None,
                space,
            );
            let tools: Vec<&Node> = scene
                .nodes
                .iter()
                .filter(|n| n.id.starts_with("harness:"))
                .collect();
            for n in &tools {
                assert!(
                    n.at.y >= band.0 && n.at.y <= band.1,
                    "{} off the band at {pane:?}",
                    n.id
                );
            }
            assert!(tightest(&tools) >= HARNESS_STEP_MIN - 0.5);
            let counted: usize = scene.overflows.iter().map(|o| o.count).sum();
            assert_eq!(tools.len() + counted, count, "{count} at {pane:?}");
            // The credential counts every connected tool, drawn or not.
            let destination = node(&scene, "destination").unwrap();
            assert_eq!(
                destination.sublabel.as_deref(),
                Some(connected_words(&words, count, false).as_str())
            );
        }
    }
}

/// `test_aToolWithoutArtworkCarriesItsInitials`: never a blank circle.
#[test]
fn a_tool_without_artwork_carries_its_initials() {
    assert_eq!(initials("Cursor"), "Cu");
    assert_eq!(initials("GitHub Copilot CLI"), "GC");
    assert_eq!(initials("Agent 3"), "A3");
    let scene = private_ai(
        Some(&many_harnesses(3)),
        Some("running"),
        HarnessState::NotConnected,
        None,
        SIZE,
    );
    for n in scene.nodes.iter().filter(|n| n.id.starts_with("harness:")) {
        assert!(
            matches!(n.kind, NodeKind::Harness { tool: Some(_), .. })
                || harness_initials(n).is_some(),
            "{} is a blank circle",
            n.id
        );
    }
    let cursor = Node::new(
        "harness:cursor",
        NodeKind::Harness {
            tool: None,
            installed: true,
        },
        Point::default(),
        13.0,
        "Cursor",
        "",
    );
    assert_eq!(harness_initials(&cursor).as_deref(), Some("Cu"));
    let claude = Node::new(
        "harness:claude",
        NodeKind::Harness {
            tool: Some(SourceKind::ClaudeCode),
            installed: true,
        },
        Point::default(),
        13.0,
        "Claude Code",
        "",
    );
    assert_eq!(
        harness_initials(&claude),
        None,
        "a tool with artwork draws it"
    );
}

/// `test_theMapWordsAreTheCores`: the credential node takes the core's
/// words, and a list with no tools says none were found.
#[test]
fn the_map_words_are_the_cores() {
    let words = words();
    assert_eq!(words.credential, "NEAR AI credential");
    assert_eq!(words.none_found, "No configured tools found.");
    let empty = private_ai(Some(&[]), Some("off"), HarnessState::Unknown, None, SIZE);
    assert_eq!(empty.nodes[0].sublabel.as_deref(), Some(words.none_found));
    assert!(empty.draws_map());
}

/// `testUnreadHarnessListWithoutManagedRoutesDrawsNoMap`: an unread list
/// is not an empty one, and draws nothing at all.
#[test]
fn an_unread_harness_list_without_managed_routes_draws_no_map() {
    let scene = private_ai(None, Some("off"), HarnessState::Unknown, None, SIZE)
        .adding_managed_routes(None);
    assert!(!scene.draws_map());
    assert!(scene.nodes.is_empty() && scene.arcs.is_empty());
}

/// The layout: the destination right of centre on the band's middle, the
/// tools in a column to its left, each joined by a curve that leaves the
/// tool level and arrives level; a tool not on this computer is faded.
#[test]
fn the_private_ai_map_is_laid_out_as_the_macs() {
    let rows = harness_rows("normalDay");
    let scene = private_ai(
        Some(&rows),
        Some("off"),
        HarnessState::NotConnected,
        None,
        SIZE,
    );
    let band = Scene::band(SIZE);
    let destination = Point::new(SIZE.width * 0.68, (band.0 + band.1) / 2.0);
    assert_eq!(Scene::destination(SIZE), destination);
    assert_eq!(Scene::harness_x(SIZE), 150.0_f64.max(SIZE.width * 0.36));
    let d = node(&scene, "destination").unwrap();
    assert_eq!((d.at, d.radius), (destination, 26.0));
    let x = Scene::harness_x(SIZE);
    for (row, arc) in rows.iter().zip(&scene.arcs) {
        let n = node(&scene, &format!("harness:{}", row.id)).unwrap();
        assert_eq!((n.at.x, n.radius), (x, 13.0));
        assert_eq!(n.label, row.name);
        assert!(n.label_beside());
        let bend = (x + destination.x) / 2.0;
        assert_eq!(arc.from, Point::new(x + 12.0, n.at.y));
        assert_eq!(arc.control1, Point::new(bend, n.at.y));
        assert_eq!(arc.control2, Point::new(bend, destination.y));
        assert_eq!(arc.to, destination);
    }
    // Centred on the destination's line, at the most line height.
    let ys: Vec<f64> = rows
        .iter()
        .map(|r| node(&scene, &format!("harness:{}", r.id)).unwrap().at.y)
        .collect();
    assert_eq!(ys[1] - ys[0], HARNESS_STEP_MAX);
    assert!((ys.iter().sum::<f64>() / ys.len() as f64 - destination.y).abs() < 1e-9);
    // Not on this computer by its own row, but the core lists it as
    // installed: drawn, faded.
    let mut missing = rows.clone();
    missing[0].installed = false;
    let listed = destinations("normalDay");
    let scene = private_ai(
        Some(&missing),
        Some("off"),
        HarnessState::NotConnected,
        Some(&listed),
        SIZE,
    );
    let n = node(&scene, "harness:claude").unwrap();
    assert_eq!(n.dim, 0.5);
    assert_eq!(
        n.kind,
        NodeKind::Harness {
            tool: Some(SourceKind::ClaudeCode),
            installed: false
        }
    );
}

/// `MapToolPile.hover`: under the pointer the pile grows taller at once,
/// wider 260 ms later and shows its names 200 ms after that; leaving, the
/// names go at once, the width 100 ms later and the height 220 ms after
/// that. Under reduced motion every stage is at once.
#[test]
fn the_pile_opens_and_closes_in_the_macs_stages() {
    assert_eq!(
        pile_steps(true, false),
        [
            (0, PileStep::Tall(true)),
            (260, PileStep::Wide(true)),
            (460, PileStep::Names(true)),
        ]
    );
    assert_eq!(
        pile_steps(false, false),
        [
            (0, PileStep::Names(false)),
            (100, PileStep::Wide(false)),
            (320, PileStep::Tall(false)),
        ]
    );
    for inside in [true, false] {
        assert!(pile_steps(inside, true).iter().all(|(at, _)| *at == 0));
    }
    let mut stage = PileStage::default();
    for (_, step) in pile_steps(true, false) {
        stage = stage.with(step);
    }
    assert_eq!(
        stage,
        PileStage {
            tall: true,
            wide: true,
            names: true
        }
    );
    // Leaving halfway through opening: the names never show, and the
    // card narrows and shortens as it closes.
    let half = PileStage::default().with(PileStep::Tall(true));
    let closing: Vec<PileStage> = pile_steps(false, false)
        .iter()
        .scan(half, |stage, (_, step)| {
            *stage = stage.with(*step);
            Some(*stage)
        })
        .collect();
    assert_eq!(closing.last(), Some(&PileStage::default()));
    assert!(closing.iter().all(|stage| !stage.names));
}

/// The pile's card and coins part-way open: the closed and open shapes at
/// 0 and 1, and in proportion between, as a SwiftUI spring moves a frame
/// and a position (`MapToolPile`).
#[test]
fn the_pile_moves_in_proportion_between_closed_and_open() {
    let stack = ToolStack {
        center: Point::new(100.0, 200.0),
        tools: vec!["tool:a".into(), "tool:b".into(), "tool:c".into()],
    };
    assert_eq!(stack.height_at(0.0), stack.height(false));
    assert_eq!(stack.height_at(1.0), stack.height(true));
    let between = stack.height(false) + (stack.height(true) - stack.height(false)) * 0.6;
    assert!((stack.height_at(0.6) - between).abs() < 1e-9);
    for index in 0..3 {
        assert_eq!(
            stack.coin_center_at(index as f64, 0.0),
            stack.coin_center(index, false)
        );
        assert_eq!(
            stack.coin_center_at(index as f64, 1.0),
            stack.coin_center(index, true)
        );
    }
    // Halfway between the first and second places, half open.
    let at = stack.coin_center_at(0.5, 0.5);
    let step = (ToolStack::STEP + ToolStack::OPEN_STEP) / 2.0;
    assert!((at.y - (200.0 - 0.5 * step)).abs() < 1e-9, "{at:?}");
    let fit = MapGeometry::new(
        Size::new(580.0, 760.0),
        1.0,
        None,
        Scene::space_for(Size::new(580.0, 760.0)),
    );
    assert_eq!(stack.card_at(&fit, 1.0, 0.0), stack.card(&fit, true, false));
    assert_eq!(stack.card_at(&fit, 0.0, 1.0), stack.card(&fit, false, true));
    let (_, _, w, _) = stack.card_at(&fit, 1.0, 0.5);
    let wide = (ToolStack::WIDTH + ToolStack::OPEN_WIDTH) / 2.0 * fit.scale;
    assert!((w - wide).abs() < 1e-9, "{w}");
}
