//! Account and session actions use the same daemon service as the NEAR AI CLI.
use super::{App, style};
use adw::prelude::*;
use serde_json::{Value, json};
use std::rc::Rc;

fn connections(tool: &str) -> Vec<&'static str> {
    if tool == "opencode" {
        vec!["api_key", "near_ai"]
    } else {
        vec!["subscription", "api_key", "near_ai"]
    }
}

fn version_recovery_tools<'a>(snapshot: &'a Value, attempted: Option<&str>) -> Vec<&'a Value> {
    snapshot["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["installed"] == true && attempted.is_none_or(|id| tool["tool"] == id))
        .collect()
}

fn text(key: &str) -> String {
    trace_commons_contributor::managed::copy::copy()[key]
        .as_str()
        .unwrap_or_default()
        .into()
}
/// The tools this shell launches: OpenCode too since #1418.
const LAUNCHED: [&str; 3] = ["claude", "codex", "opencode"];
fn can_launch(snapshot: &Value, tool: &str) -> bool {
    LAUNCHED.contains(&tool)
        && snapshot["tools"].as_array().is_none_or(|tools| {
            tools
                .iter()
                .any(|t| t["tool"] == tool && t["installed"] == true)
        })
}
fn setup_tools(snapshot: &Value) -> Vec<&'static str> {
    LAUNCHED
        .into_iter()
        .filter(|tool| can_launch(snapshot, tool))
        .collect()
}
/// The tools to offer Install for: every tool this shell launches, and only
/// while none of them is installed (Ron's review of #1419, 2026-10-15).
fn install_block(snapshot: &Value) -> Vec<Value> {
    let Some(tools) = snapshot["tools"].as_array() else {
        return Vec::new();
    };
    if !setup_tools(snapshot).is_empty() {
        return Vec::new();
    }
    tools
        .iter()
        .filter(|t| LAUNCHED.contains(&t["tool"].as_str().unwrap_or_default()))
        .cloned()
        .collect()
}
/// Whether the caption and Install belong on an account row: the tool is
/// missing from the snapshot's inventory. This keys on installation, not on
/// launchability, so an OpenCode account on a machine with OpenCode is clean.
fn tool_missing(snapshot: &Value, tool: &str) -> bool {
    snapshot["tools"].as_array().is_some_and(|tools| {
        !tools
            .iter()
            .any(|t| t["tool"] == tool && t["installed"] == true)
    })
}
/// Reconnect is hidden, not disabled, while the account's tool is missing.
fn offers_reconnect(snapshot: &Value, account: &Value) -> bool {
    account["connection"] == "subscription"
        && connections(account["tool"].as_str().unwrap_or_default()).contains(&"subscription")
        && can_launch(snapshot, account["tool"].as_str().unwrap_or_default())
}
/// `tool_unavailable` with `{tool}` filled with the tool's display name.
fn tool_unavailable(tool: &str) -> String {
    text("tool_unavailable").replace("{tool}", &text(tool))
}
/// One Install button for a tool's setup guide, named with the tool.
fn install_button(tool: &Value) -> Option<gtk::Button> {
    let id = tool["tool"].as_str().unwrap_or_default().to_owned();
    let url = tool["install_url"].as_str()?.to_owned();
    let install = button("install", move || {
        let _ =
            gtk::gio::AppInfo::launch_default_for_uri(&url, None::<&gtk::gio::AppLaunchContext>);
    });
    install.update_property(&[gtk::accessible::Property::Label(&format!(
        "{} {}",
        text("install"),
        text(&id)
    ))]);
    Some(install)
}
/// A bare refresh glyph, its tooltip and accessible name "Refresh".
fn refresh_glyph(app: &Rc<App>) -> gtk::Button {
    kept_glyph(&app.private_inference.managed_refresh, || {
        // Its icon turns once when pressed (`GlassRefreshButton`).
        let (glyph, _) = super::glass_feedback::refresh_button(&text("refresh"));
        let app = Rc::downgrade(app);
        glyph.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                refresh(&app);
            }
        });
        glyph
    })
}

/// The one glyph `slot` holds, made on first use and taken out of the
/// heading it was in, so a rebuild moves it rather than making a new one:
/// the Mac's button keeps its identity across a redraw, and with it the
/// turn its press started.
fn kept_glyph(
    slot: &std::cell::OnceCell<gtk::Button>,
    make: impl FnOnce() -> gtk::Button,
) -> gtk::Button {
    let glyph = slot.get_or_init(make).clone();
    if let Some(parent) = glyph.parent() {
        match parent.downcast::<gtk::Box>() {
            Ok(heading) => heading.remove(&glyph),
            Err(_) => glyph.unparent(),
        }
    }
    glyph
}
fn launch_accounts(snapshot: &Value) -> Vec<Value> {
    snapshot["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| can_launch(snapshot, a["tool"].as_str().unwrap_or_default()))
        .cloned()
        .collect()
}

/// A line led by its tool's tile, as the Mac's saved-account and session
/// rows are (`GlassToolTile(.tool(ManagedWords.glassTool(tool)))`).
fn tiled(tool: &str, line: String) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, style::space::S);
    row.append(&super::tool_tile::ToolTile::new(
        super::tool_tile::Kind::managed(tool),
        false,
    ));
    let label = style::body(line);
    label.set_hexpand(true);
    row.append(&label);
    row
}

/// What Red Team's row shows, from the daemon's snapshot.
#[derive(Debug, PartialEq, Eq)]
enum RedTeamRow {
    /// Nothing: an older daemon, a platform without Red Team, or aarch64.
    Hidden,
    /// The daemon runs inside a Flatpak sandbox, which has no KVM and no user
    /// session for a VM: the host service runs Red Team.
    FlatpakHost,
    /// The daemon's `red_team` row.
    Row {
        installed: bool,
        /// A `host_problem` the person fixes themselves: the copy key that
        /// explains it.
        problem_key: Option<&'static str>,
        /// The command the person runs in their own terminal, never the app.
        fix_command: Option<String>,
    },
}

/// The copy key that explains a `host_problem`; the Windows ones are another
/// shell's.
fn red_team_problem_key(problem: &str) -> Option<&'static str> {
    match problem {
        "no_kvm" => Some("red_team_no_kvm"),
        "kvm_not_accessible" => Some("red_team_kvm_access"),
        "no_user_session" => Some("red_team_no_user_session"),
        _ => None,
    }
}

fn red_team_row(snapshot: &Value, x86_64: bool) -> RedTeamRow {
    if let Some(row) = snapshot["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["tool"] == "red_team")
    {
        return RedTeamRow::Row {
            installed: row["installed"] == true,
            problem_key: row["host_problem"].as_str().and_then(red_team_problem_key),
            fix_command: row["fix_command"].as_str().map(str::to_owned),
        };
    }
    // A daemon inside the sandbox sends no row; one on the host (or an older
    // one that does not say) is not asked to explain.
    if x86_64 && snapshot["capabilities"]["daemon_on_host"] == false {
        RedTeamRow::FlatpakHost
    } else {
        RedTeamRow::Hidden
    }
}

/// The `managed_launch_prepare` request that starts Red Team on the NEAR AI
/// sign-in in the home folder (no `cwd`, as the other shells' Start), or None
/// without a sign-in entry for it.
fn red_team_start_request(snapshot: &Value, request_id: &str) -> Option<Value> {
    let entry = snapshot["sign_in_accounts"]
        .as_array()?
        .iter()
        .find(|a| a["tool"] == "red_team" && a["connection"] == "near_ai_signin")?;
    Some(json!({
        "request_id": request_id, "purpose": "coding", "tool": "red_team",
        "connection": "near_ai_signin", "account_id": entry["id"],
        "expected_generation": snapshot["generations"]["red_team"], "save_default": false,
    }))
}

/// A command the person runs themselves, with the same "Copy command" the
/// background-service dialog has; nothing here runs it.
fn command_with_copy(root: &gtk::Box, command: &str) {
    root.append(&super::service_dialog::command_label(command));
    let copy = gtk::Button::with_label(
        trace_commons_contributor::managed::copy::background_service::COPY_COMMAND,
    );
    copy.set_halign(gtk::Align::Start);
    let command = command.to_owned();
    copy.connect_clicked(move |button| button.clipboard().set_text(&command));
    root.append(&copy);
}

/// Red Team's row: its state with Install and Start, or the host problem and
/// the command that fixes it, or (inside a Flatpak) the host service's install
/// command. Words are the managed copy's.
fn red_team_section(app: &Rc<App>, root: &gtk::Box, snapshot: &Value) {
    let row = red_team_row(snapshot, cfg!(target_arch = "x86_64"));
    if row == RedTeamRow::Hidden {
        return;
    }
    root.append(&style::section(&text("red_team")));
    match row {
        RedTeamRow::Hidden => {}
        RedTeamRow::FlatpakHost => {
            style::append_body(root, text("red_team_flatpak_host"));
            command_with_copy(
                root,
                trace_commons_contributor::daemon::service::SERVICE_INSTALL_COMMAND,
            );
        }
        RedTeamRow::Row {
            installed,
            problem_key,
            fix_command,
        } => {
            style::append_meta(
                root,
                text(if installed {
                    "red_team_installed"
                } else {
                    "red_team_not_installed"
                }),
            );
            style::append_body(root, text("red_team_settings_line"));
            if let Some(key) = problem_key {
                style::append_body(root, text(key));
                if let Some(command) = &fix_command {
                    command_with_copy(root, command);
                }
            } else if !installed {
                let a = Rc::clone(app);
                root.append(&button("install", move || {
                    action(
                        &a,
                        "managed_tool_setup",
                        json!({"tool":"red_team","action":"install"}),
                        false,
                    )
                }));
            } else if let Some(request) =
                red_team_start_request(snapshot, &uuid::Uuid::new_v4().to_string())
            {
                let a = Rc::clone(app);
                root.append(&button("start_red_team", move || {
                    let mut request = request.clone();
                    request["request_id"] = json!(uuid::Uuid::new_v4());
                    action(&a, "managed_launch_prepare", request, true)
                }));
            }
        }
    }
}

fn button(key: &str, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(&text(key));
    b.connect_clicked(move |_| f());
    b
}
fn clear(root: &gtk::Box) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
}

/// Where a session opens, for the launch lines and a launch's refusal:
/// herdr while sessions are grouped there, else the chosen terminal; an
/// older daemon sends only `terminal_destination` (owner, 2026-11-03). This
/// shell runs no tool setup, so it never needs the chosen terminal alone.
fn session_destination(snapshot: &Value) -> Option<&str> {
    let capabilities = &snapshot["capabilities"];
    capabilities["launch_destination"]
        .as_str()
        .or_else(|| capabilities["terminal_destination"].as_str())
}

/// The line for a refused terminal launch. A chosen terminal that was not
/// found has its own, naming it and pointing at Settings, as does an
/// untrusted herdr socket (owner, 2026-11-03); anything else is the generic
/// launch line.
fn launch_refusal(label: &str, destination: Option<&str>) -> String {
    match label {
        "managed-terminal-not-found" => {
            let terminal = destination
                .map(str::to_owned)
                .unwrap_or_else(|| text("terminal"));
            text("terminal_not_found").replace("{terminal}", &terminal)
        }
        "managed-herdr-socket-untrusted" => text("herdr_socket_untrusted"),
        _ => text("launch_unknown"),
    }
}

/// The "Open sessions in" row a launch dialog shows (#1891, as the macOS
/// app's `asksForTerminal` and `proposedTerminal`): the installed terminals
/// as (id, name) and the position it starts on. Shown on every launch,
/// whether or not a terminal was chosen before, but only when the daemon
/// reports `terminal_chosen` (an older daemon that omits it never asks) and
/// more than one terminal is installed. It starts on the current terminal
/// when that is installed, else the first.
fn terminal_ask(snapshot: &Value) -> Option<(Vec<(String, String)>, u32)> {
    let capabilities = &snapshot["capabilities"];
    if !capabilities["terminal_chosen"].is_boolean() {
        return None;
    }
    let (choices, _) = super::terminal_choice::model(snapshot)?;
    if choices.len() < 2 {
        return None;
    }
    let current = capabilities["terminal_current"].as_str();
    let start = current
        .and_then(|id| choices.iter().position(|(choice, _)| choice == id))
        .unwrap_or(0) as u32;
    Some((choices, start))
}

/// The terminal a launch saves as the default before it starts: the one
/// picked in a dialog that asked, else none.
fn terminal_to_save(ask: Option<&[(String, String)]>, picked: u32) -> Option<String> {
    ask?.get(picked as usize).map(|(id, _)| id.clone())
}

/// Where the session opens while the dialog asks: the terminal picked in
/// it, as the macOS sheet's `destination`; else the daemon's own line, which
/// names herdr while sessions are grouped there.
fn launch_destination<'a>(
    snapshot: &'a Value,
    ask: Option<&'a [(String, String)]>,
    picked: u32,
) -> Option<&'a str> {
    // Grouped in herdr, the session opens there whatever is picked.
    let grouped = snapshot["capabilities"]["herdr_grouping"] == true;
    if grouped {
        return session_destination(snapshot);
    }
    ask.and_then(|choices| choices.get(picked as usize))
        .map(|(_, name)| name.as_str())
        .or_else(|| session_destination(snapshot))
}

fn scope_text(snapshot: &Value, ask: Option<&[(String, String)]>, picked: u32) -> String {
    text("launch_scope").replace(
        "{destination}",
        launch_destination(snapshot, ask, picked).unwrap_or_default(),
    )
}

/// Keeps the launch line on the dropdown's pick.
fn follow_pick(
    dropdown: &gtk::DropDown,
    line: &gtk::Label,
    snapshot: &Value,
    choices: Option<Vec<(String, String)>>,
) {
    let snapshot = snapshot.clone();
    let line = line.clone();
    dropdown.connect_selected_notify(move |d| {
        line.set_label(&scope_text(&snapshot, choices.as_deref(), d.selected()));
    });
}

/// The "Open sessions in" row: the core's label and a dropdown over the
/// installed terminals, starting on `start`.
fn terminal_row(choices: &[(String, String)], start: u32) -> (gtk::Box, gtk::DropDown) {
    let label_text = text("terminal_choice");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::builder()
        .label(&label_text)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    row.append(&label);
    let names: Vec<&str> = choices.iter().map(|(_, name)| name.as_str()).collect();
    let dropdown = gtk::DropDown::from_strings(&names);
    dropdown.update_property(&[gtk::accessible::Property::Label(&label_text)]);
    dropdown.set_selected(start);
    row.append(&dropdown);
    (row, dropdown)
}

fn action(app: &Rc<App>, method: &str, params: Value, terminal: bool) {
    // The terminal the launch opens in, for the not-found line.
    let destination = app.private_inference.managed_destination.borrow().clone();
    let attempted_tool = params["tool"].as_str().map(str::to_owned);
    app.call(method, params, move |app, result| match result {
        Ok(value) if terminal => {
            app.call(
                "managed_terminal_launch",
                json!({"session_id":value["session_id"],"ticket":value["ticket"]}),
                move |app, result| {
                    if let Err(label) = result {
                        app.toast(&launch_refusal(&label, destination.as_deref()));
                    }
                    refresh(app);
                },
            );
        }
        Ok(_) => refresh(app),
        Err(error) if error == "managed-unsupported-version" => {
            let (window, content) = dialog(app, attempted_tool.as_deref().unwrap_or("tools_title"));
            style::append_body(&content, text("unsupported_version"));
            let content_copy = content.clone();
            app.call("managed_snapshot", json!({}), move |_, result| {
                if let Ok(snapshot) = result {
                    for tool in version_recovery_tools(&snapshot, attempted_tool.as_deref()) {
                        style::append_body(
                            &content_copy,
                            text(tool["tool"].as_str().unwrap_or_default()),
                        );
                        if let Some(url) = tool["install_url"].as_str() {
                            content_copy.append(&gtk::LinkButton::with_label(url, &text("update")));
                        }
                    }
                }
            });
            let a = Rc::clone(app);
            let w = window.clone();
            content.append(&button("refresh", move || {
                refresh(&a);
                w.close();
            }));
            window.present();
        }
        Err(error) => app.toast(&text(refusal_key(&error))),
    });
}

/// The copy key for a refused managed action, as the macOS shell maps it:
/// no NEAR AI sign-in asks for one, a model NEAR AI does not offer says so,
/// a balance too low for a session's first calls asks for funds, and a NEAR
/// AI project that chooses a model says why; anything else is the general
/// failure.
fn refusal_key(label: &str) -> &'static str {
    match label {
        "managed-near-ai-sign-in-required" => "sign_in_needed",
        "managed-model-unavailable" => "model_unavailable",
        "managed-near-ai-balance-too-low" => "balance_too_low",
        "managed-project-chooses-model" => "project_chooses_model",
        "managed-project-sets-auto-mode-server" => "project_sets_auto_mode_server",
        _ => "action_failed",
    }
}

/// Whether a snapshot at `revision` is drawn: never one older than the last
/// drawn, which a slow answer from before a change could otherwise be.
fn accepts_revision(seen: &std::cell::Cell<u64>, revision: u64) -> bool {
    if revision < seen.get() {
        return false;
    }
    seen.set(revision);
    true
}

/// The daemon behind the window changed (a reopen: switch-over or
/// fallback), or may have restarted (`resync_required`). Revisions count per
/// daemon -- the sandbox's and the host's are separate -- so the last one
/// drawn says nothing about the next daemon's first.
pub(super) fn backend_changed(seen: &std::cell::Cell<u64>) {
    seen.set(0);
}

/// Whether a failed `managed_snapshot` still draws the start area, with the
/// unavailable line and "Set up": in a sandbox whose daemon does not answer
/// -- the host daemon installed but stopped, attached over the host link.
/// Otherwise "Start it" would be unreachable from the window.
fn draws_unreachable(error: &str, sandboxed: bool) -> bool {
    sandboxed && error == "daemon-not-running"
}

/// What the start area is drawn from when the daemon does not answer: no
/// accounts, no sessions, and no terminal or host daemon.
fn unreachable_snapshot() -> Value {
    json!({"capabilities": {"terminal_launch": false, "daemon_on_host": false}})
}

pub fn refresh(app: &Rc<App>) {
    app.call("managed_snapshot", json!({}), |app, result| {
        let snapshot = match result {
            Ok(snapshot) => {
                let revision = snapshot["revision"].as_u64().unwrap_or_default();
                if !accepts_revision(&app.private_inference.managed_revision, revision) { return; }
                // Only a real answer moves the rail's sign-in state; an
                // unreachable daemon leaves the last one standing.
                let signed_in = Some(snapshot["capabilities"]["near_ai_signin"] == true);
                if app.private_inference.near_ai_signed_in.replace(signed_in) != signed_in {
                    super::edge_surfaces::data_changed(app);
                }
                snapshot
            }
            // No revision to compare: the next answer is drawn whatever it is.
            Err(error) if draws_unreachable(&error.to_string(), sandboxed()) => {
                backend_changed(&app.private_inference.managed_revision);
                unreachable_snapshot()
            }
            Err(_) => return,
        };
        let root = &app.private_inference.managed;
        clear(root);
        root.append(&style::section(&text("title")));
        style::append_body(root, text("description"));
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let launch_app = Rc::clone(app);
        let launch_snapshot = snapshot.clone();
        // In a Flatpak without the host daemon, Launch offers the background
        // service until "Not now" (owner-approved flow, 2026-11-17).
        let sandboxed = sandboxed();
        let has_accounts = !launch_accounts(&snapshot).is_empty();
        let launch = button("launch", move || {
            match crate::service::launch_press(&launch_snapshot, sandboxed, has_accounts, super::service_dialog::dismissed()) {
                crate::service::LaunchPress::Open => launch_dialog(&launch_app, &launch_snapshot),
                crate::service::LaunchPress::OfferService => offer_service(&launch_app, crate::service::Trigger::Launch),
                crate::service::LaunchPress::Unavailable => {}
            }
        });
        launch.set_sensitive(crate::service::launch_press(&snapshot, sandboxed, has_accounts, super::service_dialog::dismissed()) != crate::service::LaunchPress::Unavailable);
        controls.append(&launch);
        let add_app = Rc::clone(app);
        let terminal = snapshot["capabilities"]["terminal_launch"] == true;
        let tools = setup_tools(&snapshot);
        // Hidden, not disabled, with no tool to add an account for.
        if !tools.is_empty() {
            controls.append(&button("add", move || add_dialog(&add_app, terminal, tools.clone())));
        }
        root.append(&controls);
        let missing = install_block(&snapshot);
        if !missing.is_empty() {
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            heading.append(&style::section(&text("tools_title")));
            heading.append(&refresh_glyph(app));
            root.append(&heading);
            style::append_body(root, text("tools_empty"));
            for tool in &missing {
                style::append_body(root, text(tool["tool"].as_str().unwrap_or_default()));
                if let Some(install) = install_button(tool) { root.append(&install); }
            }
        }
        let destination = session_destination(&snapshot);
        *app.private_inference.managed_destination.borrow_mut() = destination.map(str::to_owned);
        style::append_meta(root, destination.map(|d| text("terminal_scope").replace("{destination}", d)).unwrap_or_else(|| text("terminal_unavailable")));
        if crate::service::offers_set_up(&snapshot, sandboxed) {
            let a = Rc::clone(app);
            let set_up = gtk::Button::with_label(trace_commons_contributor::managed::copy::background_service::SET_UP);
            set_up.set_halign(gtk::Align::Start);
            set_up.connect_clicked(move |_| offer_service(&a, crate::service::Trigger::SetUp));
            root.append(&set_up);
        }
        red_team_section(app, root, &snapshot);
        for account in snapshot["accounts"].as_array().into_iter().flatten() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            row.append(&tiled(account["tool"].as_str().unwrap_or_default(), format!("{} · {} · {} · {}", account["label"].as_str().unwrap_or_default(), text(account["tool"].as_str().unwrap_or_default()), text(account["connection"].as_str().unwrap_or_default()), account["auth_state"].as_str().unwrap_or_default())));
            let tool = account["tool"].as_str().unwrap_or_default();
            if tool_missing(&snapshot, tool) {
                style::append_meta(&row, tool_unavailable(tool));
                let availability = snapshot["tools"].as_array().into_iter().flatten().find(|t| t["tool"] == tool);
                if let Some(install) = availability.and_then(install_button) { row.append(&install); }
            }
            let held = snapshot["sessions"].as_array().into_iter().flatten().any(|s| s["account_id"] == account["id"] && s["state"] != "exited" && s["state"] != "failed");
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let a = Rc::clone(app); let account_copy = account.clone();
            let generation = snapshot["generations"][account["tool"].as_str().unwrap_or_default()].clone();
            controls.append(&button("use_default", move || action(&a, "managed_select", json!({"selection":{"tool":account_copy["tool"],"connection":account_copy["connection"],"account_id":account_copy["id"],"generation":generation},"expected_generation":generation}), false)));
            let a = Rc::clone(app); let account_copy = account.clone();
            controls.append(&button("rename_short", move || rename_dialog(&a, &account_copy)));
            if offers_reconnect(&snapshot, account) {
                let a = Rc::clone(app); let id = account["id"].clone();
                let reconnect = button("reconnect", move || action(&a, "managed_account_reconnect", json!({"account_id":id}), true));
                reconnect.set_sensitive(!held && terminal); controls.append(&reconnect);
            }
            let a = Rc::clone(app); let id = account["id"].clone();
            let remove = button("remove", move || {
                let (window, content) = dialog(&a, "remove_question");
                style::append_body(&content, text("remove_description"));
                let a = Rc::clone(&a); let id = id.clone(); let w = window.clone();
                content.append(&button("remove", move || { action(&a, "managed_account_remove", json!({"account_id":id}), false); w.close(); }));
                window.present();
            });
            remove.set_sensitive(!held); controls.append(&remove); row.append(&controls); root.append(&row);
        }
        if let Some(sessions) = snapshot["sessions"].as_array() {
            if sessions.is_empty() { style::append_body(root, text("empty")); }
            for session in sessions {
                let project = if session["purpose"] == "login" { text("sign_in") } else { session["project_label"].as_str().unwrap_or_default().into() };
                root.append(&tiled(session["tool"].as_str().unwrap_or_default(), format!("{project} · {} · {} · {} · {}", text(session["tool"].as_str().unwrap_or_default()), session["account_label"].as_str().unwrap_or_default(), text(session["connection"].as_str().unwrap_or_default()), session["state"].as_str().unwrap_or_default())));
                if let Some(model) = session["model"].as_str() { style::append_meta(root, format!("{}: {model}", text("model"))); }
                if session["tool"] == "opencode" { style::append_meta(root, text(&format!("collection_{}", session["collection"].as_str().unwrap_or("off")))); }
                if session["state"] == "exited" || session["state"] == "failed" {
                    let a = Rc::clone(app); let id = session["id"].clone();
                    root.append(&button("dismiss", move || action(&a, "managed_session_dismiss", json!({"session_id":id}), false)));
                }
            }
        }
        root.append(&style::section(&text("global_title")));
        style::append_body(root, text("global_scope"));
    });
}

#[cfg(test)]
thread_local! {
    /// A mounted test's stand-in for `/.flatpak-info`.
    static TEST_SANDBOXED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether this window runs in a Flatpak sandbox.
fn sandboxed() -> bool {
    #[cfg(test)]
    if let Some(sandboxed) = TEST_SANDBOXED.with(std::cell::Cell::get) {
        return sandboxed;
    }
    trace_commons_contributor::daemon::host_link::runs_sandboxed()
}

/// Look at the background service and show its dialog when `trigger` calls
/// for one. Ended either way, the Private AI area is drawn again: after
/// "Not now" Launch goes quiet; after a switch-over it reads the host
/// daemon (the reopen already refreshed everything).
fn offer_service(app: &Rc<App>, trigger: crate::service::Trigger) {
    app.service_state(move |app, state| {
        let Some(dialog) =
            crate::service::prompt(trigger, state, super::service_dialog::dismissed())
        else {
            refresh(app);
            return;
        };
        let host: Rc<dyn super::service_dialog::ServiceHost> =
            Rc::new(super::service_dialog::AppHost(Rc::downgrade(app)));
        let weak = Rc::downgrade(app);
        super::service_dialog::present(
            Some(app.window.upcast_ref()),
            None,
            dialog,
            host,
            move |_| {
                if let Some(app) = weak.upgrade() {
                    refresh(&app);
                }
            },
        );
    });
}

fn dialog(app: &Rc<App>, title: &str) -> (gtk::Window, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let window = gtk::Window::builder()
        .title(text(title))
        .transient_for(&app.window)
        .modal(true)
        .default_width(500)
        .child(&content)
        .build();
    let w = window.clone();
    content.append(&button("cancel", move || w.close()));
    (window, content)
}

fn rename_dialog(app: &Rc<App>, account: &Value) {
    let (window, content) = dialog(app, "rename");
    let entry = gtk::Entry::builder()
        .text(account["label"].as_str().unwrap_or_default())
        .placeholder_text(text("label"))
        .build();
    content.append(&entry);
    let a = Rc::clone(app);
    let id = account["id"].clone();
    let w = window.clone();
    content.append(&button("save", move || {
        action(
            &a,
            "managed_account_rename",
            json!({"account_id":id,"label":entry.text().as_str()}),
            false,
        );
        w.close();
    }));
    window.present();
}

fn add_dialog(app: &Rc<App>, terminal: bool, tools: Vec<&'static str>) {
    if tools.is_empty() {
        return;
    }
    let (window, content) = dialog(app, "save_title");
    let tool_labels: Vec<_> = tools.iter().map(|id| text(id)).collect();
    let tool =
        gtk::DropDown::from_strings(&tool_labels.iter().map(String::as_str).collect::<Vec<_>>());
    let kinds = Rc::new(std::cell::RefCell::new(connections(tools[0])));
    let labels: Vec<_> = kinds.borrow().iter().map(|k| text(k)).collect();
    let connection =
        gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let label = gtk::Entry::builder()
        .placeholder_text(text("label_placeholder"))
        .build();
    let key = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    key.set_visible(kinds.borrow()[0] != "subscription");
    let k = key.clone();
    let ks = Rc::clone(&kinds);
    connection.connect_selected_notify(move |c| {
        k.set_visible(ks.borrow().get(c.selected() as usize) != Some(&"subscription"))
    });
    let c = connection.clone();
    let ks = Rc::clone(&kinds);
    let ts = tools.clone();
    let k = key.clone();
    tool.connect_selected_notify(move |t| {
        let Some(selected) = ts.get(t.selected() as usize) else {
            return;
        };
        *ks.borrow_mut() = connections(selected);
        let labels: Vec<_> = ks.borrow().iter().map(|k| text(k)).collect();
        c.set_model(Some(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        c.set_selected(0);
        k.set_visible(ks.borrow()[0] != "subscription");
    });
    content.append(&tool);
    content.append(&connection);
    content.append(&label);
    content.append(&key);
    let open_code_scope = gtk::Label::new(Some(&text("opencode_scope")));
    open_code_scope.set_wrap(true);
    open_code_scope.set_visible(tools[0] == "opencode");
    let scope = open_code_scope.clone();
    let ts = tools.clone();
    tool.connect_selected_notify(move |t| {
        scope.set_visible(
            ts.get(t.selected() as usize)
                .is_some_and(|t| *t == "opencode"),
        )
    });
    content.append(&open_code_scope);
    let login = gtk::Label::new(Some(&text("login_description")));
    login.set_wrap(true);
    login.set_visible(kinds.borrow()[0] == "subscription");
    let l = login.clone();
    let ks = Rc::clone(&kinds);
    connection.connect_selected_notify(move |c| {
        l.set_visible(ks.borrow().get(c.selected() as usize) == Some(&"subscription"))
    });
    let l = login.clone();
    let ks = Rc::clone(&kinds);
    tool.connect_selected_notify(move |_| l.set_visible(ks.borrow()[0] == "subscription"));
    content.append(&login);
    style::append_meta(&content, text("key_storage"));
    let a = Rc::clone(app);
    let w = window.clone();
    content.append(&button("save", move || {
        let Some(kind) = kinds.borrow().get(connection.selected() as usize).copied() else { return; };
        let subscription = kind == "subscription";
        if label.text().trim().is_empty() || (subscription && !terminal) { a.toast(&text("terminal_unavailable")); return; }
        let secret = key.text().to_string();
        if !subscription && secret.is_empty() { return; }
        a.call("managed_account_add", json!({"tool":tools[tool.selected() as usize],"connection":kind,"label":label.text().as_str()}), move |app, result| {
            if let Ok(account) = result {
                if subscription { action(app, "managed_account_reconnect", json!({"account_id":account["id"]}), true); }
                else { action(app, "managed_account_set_key", json!({"account_id":account["id"],"key":secret}), false); }
            } else { app.toast(&text("action_failed")); }
        });
        key.set_text(""); w.close();
    }));
    window.present();
}

/// Where Launch sends the folder it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchRoute {
    /// A switch-over is under way: where the launch would land is not
    /// known, so nothing is sent.
    Refuse,
    /// Sent as it is.
    AsIs,
    /// A document-portal folder bound for the host daemon: sent as the
    /// host folder it stands for, or not at all.
    MapToHost,
}

/// [`LaunchRoute`] for `cwd`, from `target` (`Worker::declaration_target`:
/// where a call made now would land), as `roots::declare_to_daemon`
/// decides for a folder declaration. The snapshot the dialog opened with
/// may predate a switch-over, so it is not asked.
fn launch_route(
    target: Option<bool>,
    cwd: &std::path::Path,
    doc_root: Option<&std::path::Path>,
) -> LaunchRoute {
    match target {
        None => LaunchRoute::Refuse,
        Some(on_host) if crate::document_portal::needs_mapping_in(cwd, on_host, doc_root) => {
            LaunchRoute::MapToHost
        }
        Some(_) => LaunchRoute::AsIs,
    }
}

/// Whether a folder mapped to its host path may still go: only while the
/// window is on the host daemon once the portal has answered. A switch-over
/// meanwhile would send a host path to the sandbox's daemon.
fn mapped_launch_proceeds(target: Option<bool>) -> bool {
    target == Some(true)
}

/// The `cwd` a launch sends for a folder the portal mapped to its host
/// path. `None` for a path that is not UTF-8: the wire carries a string, and
/// a lossy one would name a different folder.
fn launch_cwd(host: &std::path::Path) -> Option<Value> {
    host.to_str().map(|cwd| json!(cwd))
}

fn launch_dialog(app: &Rc<App>, snapshot: &Value) {
    let accounts = launch_accounts(snapshot);
    if accounts.is_empty() {
        return;
    }
    let (window, content) = dialog(app, "launch_title");
    let labels: Vec<_> = accounts
        .iter()
        .map(|a| {
            format!(
                "{} · {} · {}",
                text(a["tool"].as_str().unwrap_or_default()),
                a["label"].as_str().unwrap_or_default(),
                text(a["connection"].as_str().unwrap_or_default())
            )
        })
        .collect();
    let picker =
        gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    // The folder sliding into the terminal, with the picked account's tool
    // in it (`ManagedFolderHandoff`); it follows the picker.
    let tool_of = |index: usize| {
        super::tool_tile::Kind::managed(
            accounts
                .get(index)
                .and_then(|a| a["tool"].as_str())
                .unwrap_or_default(),
        )
    };
    let handoff = Rc::new(super::folder_handoff::FolderHandoff::new(tool_of(0)));
    content.append(&handoff.area);
    {
        let accounts = accounts.clone();
        let handoff = Rc::clone(&handoff);
        picker.connect_selected_notify(move |p| {
            handoff.set_tool(super::tool_tile::Kind::managed(
                accounts
                    .get(p.selected() as usize)
                    .and_then(|a| a["tool"].as_str())
                    .unwrap_or_default(),
            ));
        });
    }
    content.append(&picker);
    let folder = gtk::Entry::builder()
        .placeholder_text(text("project_placeholder"))
        .build();
    content.append(&folder);
    let model = gtk::Entry::builder()
        .placeholder_text(text("model"))
        .build();
    let collect = gtk::CheckButton::with_label(&text("collect_traces"));
    collect.set_active(false);
    let scope = gtk::Label::new(Some(&format!(
        "{} {}",
        text("opencode_scope"),
        text("collection_scope")
    )));
    scope.set_wrap(true);
    let update = {
        let accounts = accounts.clone();
        let model = model.clone();
        let collect = collect.clone();
        let scope = scope.clone();
        move |index: usize| {
            let visible = accounts.get(index).is_some_and(|a| a["tool"] == "opencode");
            model.set_visible(visible);
            collect.set_visible(visible);
            scope.set_visible(visible);
            collect.set_active(false);
        }
    };
    update(0);
    picker.connect_selected_notify(move |p| update(p.selected() as usize));
    content.append(&model);
    content.append(&collect);
    content.append(&scope);
    // "Open sessions in", on every launch while more than one terminal is
    // installed (#1891). The label is the core's.
    let terminals = terminal_ask(snapshot);
    let terminal_picker = terminals.as_ref().map(|(choices, start)| {
        let (row, dropdown) = terminal_row(choices, *start);
        content.append(&row);
        dropdown
    });
    let terminal_choices: Option<Vec<(String, String)>> = terminals.map(|(choices, _)| choices);
    let w = window.clone();
    let f = folder.clone();
    content.append(&button("choose_folder", move || {
        let chooser = gtk::FileChooserNative::new(
            Some(&text("choose_folder")),
            Some(&w),
            gtk::FileChooserAction::SelectFolder,
            None,
            None,
        );
        let f = f.clone();
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept
                && let Some(path) = chooser.file().and_then(|f| f.path())
            {
                f.set_text(&path.to_string_lossy());
            }
            chooser.destroy();
        });
        chooser.show();
    }));
    // The line follows the pick, as on macOS.
    let scope_line = style::meta(scope_text(
        snapshot,
        terminal_choices.as_deref(),
        terminal_picker.as_ref().map_or(0, |d| d.selected()),
    ));
    content.append(&scope_line);
    if let Some(dropdown) = &terminal_picker {
        follow_pick(dropdown, &scope_line, snapshot, terminal_choices.clone());
    }
    let a = Rc::clone(app);
    let generations = snapshot["generations"].clone();
    // A folder from the sandboxed chooser is a document-portal path; a
    // daemon on the host gets the folder it stands for, or no launch
    // (`launch_route`).
    let w = window.clone();
    content.append(&button("launch_short", move || {
        let Some(account) = accounts.get(picker.selected() as usize) else { return; };
        let opencode = account["tool"] == "opencode";
        if opencode && model.text().trim().is_empty() { a.toast(&text("model_required")); return; }
        let cwd = folder.text().to_string();
        let mut request = json!({"request_id":uuid::Uuid::new_v4(),"purpose":"coding","tool":account["tool"],"connection":account["connection"],"account_id":account["id"],"cwd":cwd,"expected_generation":generations[account["tool"].as_str().unwrap_or_default()],"save_default":false,"model":if opencode { Some(model.text().trim().to_owned()) } else { None },"collect_traces":opencode && collect.is_active()});
        // The picked terminal is saved before the launch; a refused save
        // stops the launch.
        let picked = terminal_picker.as_ref().map_or(0, |d| d.selected());
        let terminal = terminal_to_save(terminal_choices.as_deref(), picked);
        let launch = move |a: &Rc<App>, request: Value| match terminal {
            Some(terminal) => a.call(
                "set_managed_terminal",
                json!({ "terminal": terminal }),
                move |app, result| {
                    if result.is_err() {
                        app.toast(&text("action_failed"));
                        return;
                    }
                    action(app, "managed_launch_prepare", request, true);
                },
            ),
            None => action(a, "managed_launch_prepare", request, true),
        };
        let route = launch_route(
            a.worker.declaration_target(),
            std::path::Path::new(&cwd),
            crate::document_portal::doc_root().as_deref(),
        );
        if route == LaunchRoute::Refuse {
            a.toast(&text("action_failed"));
        } else if route == LaunchRoute::MapToHost {
            // One D-Bus round trip, off the GTK thread.
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(crate::document_portal::host_path(
                    std::path::Path::new(&cwd),
                    true,
                ));
            });
            let a = Rc::clone(&a);
            gtk::glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(_)) if !mapped_launch_proceeds(a.worker.declaration_target()) => {
                        a.toast(&text("action_failed"))
                    }
                    Ok(Ok(host)) => match launch_cwd(&host) {
                        Some(cwd) => {
                            request["cwd"] = cwd;
                            launch(&a, request);
                        }
                        None => a.toast(&text("action_failed")),
                    },
                    _ => a.toast(&text("action_failed")),
                }
            });
        } else {
            launch(&a, request);
        }
        w.close();
    }));
    window.present();
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod mounted_tests {
    use super::*;
    use crate::worker::{Job, Outcome, Worker};
    use trace_commons_contributor::managed::copy::background_service as words;

    fn texts(root: &gtk::Widget, out: &mut Vec<(String, bool)>) {
        if let Some(label) = root.downcast_ref::<gtk::Label>() {
            out.push((label.label().to_string(), true));
        }
        if let Some(button) = root.downcast_ref::<gtk::Button>() {
            out.push((
                button.label().map(|l| l.to_string()).unwrap_or_default(),
                button.is_sensitive(),
            ));
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            texts(&widget, out);
            child = widget.next_sibling();
        }
    }

    /// In a sandbox whose daemon is not the host daemon, the start area
    /// keeps its unavailable line and adds "Set up"; Launch offers the
    /// service until "Not now", then goes quiet while "Set up" stays.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_sandbox_start_area_shows_unavailable_and_set_up() {
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let application = adw::Application::builder()
            .application_id("ai.tracecommons.ServiceSetUpTest")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        application
            .register(None::<&gtk::gio::Cancellable>)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("tc-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (worker, jobs, results) = Worker::fixture(dir.clone());
        let app = App::build(&application, worker);
        TEST_SANDBOXED.with(|s| s.set(Some(true)));
        let snapshot = json!({
            "revision": 1,
            "accounts": [{"id": "a1", "tool": "claude", "connection": "near_ai", "label": "A", "auth_state": "ready"}],
            "sessions": [],
            "capabilities": {"terminal_launch": false, "daemon_on_host": false},
        });
        let render = |dismissed: bool| -> Vec<(String, bool)> {
            crate::ui::service_dialog::set_dismissed(dismissed);
            app.private_inference.managed_revision.set(0);
            // Without this the loop below can read the previous draw, which
            // has "Set up" too, before the new answer is drawn.
            clear(&app.private_inference.managed);
            refresh(&app);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                while let Ok((id, job)) = jobs.try_recv() {
                    let reply = match job {
                        Job::Call { method, .. } if method == "managed_snapshot" => {
                            Ok(snapshot.clone())
                        }
                        _ => Err("synthetic-refused".to_string()),
                    };
                    let _ = results.try_send((id, Outcome::Call(reply.map_err(Into::into))));
                }
                let mut out = Vec::new();
                texts(app.private_inference.managed.upcast_ref(), &mut out);
                if out.iter().any(|(t, _)| t == words::SET_UP) {
                    return out;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the start area was not drawn"
                );
                context.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        let shown = render(false);
        assert!(
            shown
                .iter()
                .any(|(t, _)| *t == text("terminal_unavailable"))
        );
        assert!(shown.iter().any(|(t, on)| *t == words::SET_UP && *on));
        assert!(
            shown.iter().any(|(t, on)| *t == text("launch") && *on),
            "Launch offers the service"
        );
        let shown = render(true);
        assert!(
            shown.iter().any(|(t, on)| *t == words::SET_UP && *on),
            "Set up stays after Not now"
        );
        assert!(
            shown.iter().any(|(t, on)| *t == text("launch") && !*on),
            "Launch is quiet after Not now"
        );
        TEST_SANDBOXED.with(|s| s.set(None));
        crate::ui::service_dialog::set_dismissed(false);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A rebuild of the list moves the one refresh glyph into the new
    /// heading, and the turn its press started keeps going there.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_the_refresh_glyph_keeps_its_turn_across_a_rebuild() {
        gtk::init().expect("GTK display unavailable");
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(true);
        let slot = std::cell::OnceCell::new();
        let made = std::cell::Cell::new(0);
        let make = || {
            made.set(made.get() + 1);
            super::super::glass_feedback::refresh_button("Refresh").0
        };
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let window = gtk::Window::new();
        window.set_child(Some(&root));
        window.present();
        let first_heading = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let glyph = kept_glyph(&slot, make);
        first_heading.append(&glyph);
        root.append(&first_heading);
        let context = gtk::glib::MainContext::default();
        let pump_for = |ms: u64| {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                context.iteration(false);
            }
        };
        pump_for(100);
        glyph.emit_clicked();
        pump_for(100);
        // The answer lands: the list is cleared and drawn again.
        clear(&root);
        let second_heading = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let again = kept_glyph(&slot, make);
        second_heading.append(&again);
        root.append(&second_heading);
        assert_eq!(again, glyph, "the rebuild made a new glyph");
        assert_eq!(made.get(), 1);
        assert_eq!(
            again.parent().as_ref(),
            Some(second_heading.upcast_ref::<gtk::Widget>())
        );
        let turn = again
            .child()
            .and_then(|c| c.downcast::<super::super::glass_feedback::Turn>().ok())
            .expect("the glyph's turn");
        pump_for(150);
        assert!(
            turn.angle() > 0.0,
            "the press's turn stopped at the rebuild"
        );
        window.close();
    }
}

#[cfg(test)]
mod tests {
    /// Launch decides where a folder goes as a folder declaration does
    /// (`roots::declare_to_daemon`): from where a call made now would land,
    /// not from the snapshot the dialog opened with. Refused while a
    /// switch-over is under way; a portal folder is mapped only for the
    /// host daemon; and a mapped folder goes only if the window is still on
    /// the host once the portal answers.
    #[test]
    fn launch_follows_the_switch_over_like_a_declaration() {
        use super::LaunchRoute;
        let root = std::path::Path::new("/run/user/1000/doc");
        let portal = std::path::Path::new("/run/user/1000/doc/a1/project");
        let host = std::path::Path::new("/home/u/project");
        let route = |target, cwd| super::launch_route(target, cwd, Some(root));
        assert_eq!(route(None, portal), LaunchRoute::Refuse);
        assert_eq!(route(None, host), LaunchRoute::Refuse);
        assert_eq!(route(Some(true), portal), LaunchRoute::MapToHost);
        assert_eq!(route(Some(true), host), LaunchRoute::AsIs);
        assert_eq!(route(Some(false), portal), LaunchRoute::AsIs);
        assert_eq!(route(Some(false), host), LaunchRoute::AsIs);
        assert_eq!(
            super::launch_route(Some(true), portal, None),
            LaunchRoute::AsIs
        );
        // After the portal answers.
        assert!(super::mapped_launch_proceeds(Some(true)));
        assert!(!super::mapped_launch_proceeds(Some(false)));
        assert!(!super::mapped_launch_proceeds(None));
    }

    /// A mapped host folder that is not UTF-8 is refused, never sent with
    /// U+FFFD in place of its bytes: the host daemon would open a session in
    /// a folder that does not exist, or in a different one.
    #[cfg(unix)]
    #[test]
    fn a_host_folder_that_is_not_utf8_is_refused_not_mangled() {
        use std::os::unix::ffi::OsStrExt;
        let bad = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/home/u/caf\xe9"));
        assert_eq!(super::launch_cwd(bad), None);
        assert_eq!(
            super::launch_cwd(std::path::Path::new("/home/u/café")),
            Some(json!("/home/u/café"))
        );
    }

    /// Owner, 2026-11-03: a session grouped in herdr opens there, so the
    /// launch lines and a launch's refusal name herdr; an older daemon sends
    /// only `terminal_destination`, which then names where sessions open.
    #[test]
    fn a_launch_names_where_the_session_opens() {
        let grouped = serde_json::json!({"capabilities":{
            "terminal_destination":"Konsole","launch_destination":"herdr"}});
        assert_eq!(session_destination(&grouped), Some("herdr"));
        let older = serde_json::json!({"capabilities":{"terminal_destination":"Konsole"}});
        assert_eq!(session_destination(&older), Some("Konsole"));
        assert_eq!(
            session_destination(&serde_json::json!({"capabilities":{}})),
            None
        );
    }

    use super::*;

    fn two_terminals(chosen: serde_json::Value, current: serde_json::Value) -> Value {
        json!({"capabilities":{
            "terminal_chosen": chosen,
            "terminal_current": current,
            "terminal_choices":[
                {"id":"gnome_terminal","name":"GNOME Terminal"},
                {"id":"konsole","name":"Konsole"}
            ]
        }})
    }

    /// #1891: asked on every launch once the daemon reports `terminal_chosen`
    /// (true or false) and more than one terminal is installed, starting on
    /// the current one, else the first.
    #[test]
    fn a_launch_asks_for_the_terminal_on_every_launch_with_two_installed() {
        for chosen in [json!(true), json!(false)] {
            let (choices, start) = terminal_ask(&two_terminals(chosen, json!("konsole"))).unwrap();
            assert_eq!(choices.len(), 2);
            assert_eq!(start, 1);
        }
        assert_eq!(
            terminal_ask(&two_terminals(json!(false), json!(null)))
                .unwrap()
                .1,
            0
        );
        assert_eq!(
            terminal_ask(&two_terminals(json!(true), json!("ghostty")))
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn a_launch_does_not_ask_with_one_terminal_or_an_older_daemon() {
        let one = json!({"capabilities":{"terminal_chosen":false,
            "terminal_choices":[{"id":"konsole","name":"Konsole"}]}});
        assert_eq!(terminal_ask(&one), None);
        // An older daemon sends no terminal_chosen.
        let mut older = two_terminals(json!(true), json!("konsole"));
        older["capabilities"]
            .as_object_mut()
            .unwrap()
            .remove("terminal_chosen");
        assert_eq!(terminal_ask(&older), None);
        assert_eq!(
            terminal_ask(&json!({"capabilities":{"terminal_chosen":true}})),
            None
        );
        assert_eq!(terminal_ask(&json!({})), None);
    }

    /// The launch line names the terminal picked in the dialog (as the
    /// macOS sheet's `destination`), else where the daemon says.
    #[test]
    fn the_launch_line_follows_the_picked_terminal() {
        let snapshot = two_terminals(json!(true), json!("konsole"));
        let (choices, _) = terminal_ask(&snapshot).unwrap();
        assert_eq!(
            launch_destination(&snapshot, Some(&choices), 0),
            Some("GNOME Terminal")
        );
        assert_eq!(
            launch_destination(&snapshot, Some(&choices), 1),
            Some("Konsole")
        );
        let mut older = snapshot.clone();
        older["capabilities"]["terminal_destination"] = json!("Konsole");
        assert_eq!(launch_destination(&older, None, 0), Some("Konsole"));
        assert_eq!(
            launch_destination(&older, Some(&choices), 9),
            Some("Konsole")
        );
        assert_eq!(launch_destination(&json!({}), None, 0), None);
    }

    /// Owner, 2026-11-03: while sessions are grouped in herdr the line names
    /// herdr whatever terminal is picked, as the daemon's own line does.
    #[test]
    fn the_launch_line_names_herdr_while_grouped() {
        let mut snapshot = two_terminals(json!(true), json!("konsole"));
        snapshot["capabilities"]["launch_destination"] = json!("herdr");
        snapshot["capabilities"]["herdr_grouping"] = json!(true);
        let (choices, _) = terminal_ask(&snapshot).unwrap();
        for picked in [0, 1] {
            assert_eq!(
                launch_destination(&snapshot, Some(&choices), picked),
                Some("herdr")
            );
        }
        snapshot["capabilities"]["herdr_grouping"] = json!(false);
        snapshot["capabilities"]["launch_destination"] = json!("Konsole");
        assert_eq!(
            launch_destination(&snapshot, Some(&choices), 0),
            Some("GNOME Terminal")
        );
    }

    /// Needs a display: the row draws the core's label over the installed
    /// apps and starts on the current one.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_launch_dialog_row_starts_on_the_current_terminal() {
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let snapshot = two_terminals(json!(true), json!("konsole"));
        let (choices, start) = terminal_ask(&snapshot).unwrap();
        let (row, dropdown) = terminal_row(&choices, start);
        assert_eq!(dropdown.selected(), 1);
        let label = row.first_child().and_downcast::<gtk::Label>().unwrap();
        assert_eq!(label.label(), text("terminal_choice"));
        let model = dropdown.model().unwrap();
        assert_eq!(model.n_items(), 2);
        dropdown.set_selected(0);
        assert_eq!(
            terminal_to_save(Some(&choices), dropdown.selected()).as_deref(),
            Some("gnome_terminal")
        );
        // The launch line follows the dropdown, and holds on herdr while
        // sessions are grouped there.
        let line = gtk::Label::new(None);
        follow_pick(&dropdown, &line, &snapshot, Some(choices.clone()));
        dropdown.set_selected(1);
        assert_eq!(
            line.label().as_str(),
            scope_text(&snapshot, Some(&choices), 1).as_str()
        );
        assert!(line.label().contains("Konsole"));
        dropdown.set_selected(0);
        assert!(line.label().contains("GNOME Terminal"));
        let mut grouped = snapshot.clone();
        grouped["capabilities"]["launch_destination"] = json!("herdr");
        grouped["capabilities"]["herdr_grouping"] = json!(true);
        let line = gtk::Label::new(None);
        follow_pick(&dropdown, &line, &grouped, Some(choices.clone()));
        dropdown.set_selected(1);
        assert!(line.label().contains("herdr"));
    }

    #[test]
    fn only_an_asked_launch_saves_the_picked_terminal() {
        let (choices, _) = terminal_ask(&two_terminals(json!(true), json!("konsole"))).unwrap();
        assert_eq!(
            terminal_to_save(Some(&choices), 0).as_deref(),
            Some("gnome_terminal")
        );
        assert_eq!(
            terminal_to_save(Some(&choices), 1).as_deref(),
            Some("konsole")
        );
        assert_eq!(terminal_to_save(Some(&choices), 5), None);
        assert_eq!(terminal_to_save(None, 1), None);
    }

    #[test]
    fn a_terminal_not_found_names_it_and_points_at_settings() {
        assert_eq!(
            launch_refusal("managed-terminal-not-found", Some("Konsole")),
            "Konsole wasn't found on this computer. Choose another in Settings."
        );
        assert_eq!(
            launch_refusal("managed-terminal-not-found", None),
            "your terminal wasn't found on this computer. Choose another in Settings."
        );
        assert_eq!(
            launch_refusal("managed-launch-unknown", Some("Konsole")),
            super::text("launch_unknown")
        );
    }

    /// Owner, 2026-11-03: something at herdr's socket path this account
    /// cannot trust has its own line, as on macOS.
    #[test]
    fn an_untrusted_herdr_socket_has_its_own_line() {
        assert_eq!(
            launch_refusal("managed-herdr-socket-untrusted", Some("herdr")),
            "herdr's connection file can't be trusted. Remove it or check its permissions."
        );
    }

    #[test]
    fn refusals_map_to_the_macos_lines() {
        assert_eq!(
            refusal_key("managed-near-ai-balance-too-low"),
            "balance_too_low"
        );
        assert_eq!(
            refusal_key("managed-near-ai-sign-in-required"),
            "sign_in_needed"
        );
        assert_eq!(
            refusal_key("managed-model-unavailable"),
            "model_unavailable"
        );
        assert_eq!(
            refusal_key("managed-project-chooses-model"),
            "project_chooses_model"
        );
        assert_eq!(
            refusal_key("managed-project-sets-auto-mode-server"),
            "project_sets_auto_mode_server"
        );
        for key in [
            "balance_too_low",
            "sign_in_needed",
            "model_unavailable",
            "project_chooses_model",
            "project_sets_auto_mode_server",
        ] {
            assert!(!super::text(key).is_empty(), "{key}");
        }
        assert_eq!(refusal_key("managed-conflict"), "action_failed");
    }

    #[test]
    fn version_recovery_targets_claude_and_codex() {
        let snapshot = json!({"tools":[{"tool":"claude","installed":true},{"tool":"codex","installed":true},{"tool":"opencode","installed":false}]});
        for attempted in ["claude", "codex"] {
            let tools = version_recovery_tools(&snapshot, Some(attempted));
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0]["tool"], attempted);
        }
        assert_eq!(version_recovery_tools(&snapshot, None).len(), 2);
    }

    #[test]
    fn opencode_has_no_subscription() {
        assert_eq!(connections("opencode"), vec!["api_key", "near_ai"]);
        assert_eq!(connections("codex")[0], "subscription");
    }

    #[test]
    fn availability_filters_launch_without_removing_saved_accounts() {
        let snapshot = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":true},{"tool":"opencode","installed":true}],
            "accounts":[{"tool":"claude"},{"tool":"codex"},{"tool":"opencode"}]});
        assert_eq!(setup_tools(&snapshot), vec!["codex", "opencode"]);
        assert_eq!(
            launch_accounts(&snapshot),
            vec![json!({"tool":"codex"}), json!({"tool":"opencode"})]
        );
        assert_eq!(snapshot["accounts"].as_array().unwrap().len(), 3);
        // GTK launches OpenCode since #1418.
        assert!(can_launch(&snapshot, "opencode"));
    }

    #[test]
    fn older_snapshot_retains_managed_tools_but_empty_inventory_does_not() {
        assert_eq!(setup_tools(&json!({})), vec!["claude", "codex", "opencode"]);
        assert!(setup_tools(&json!({"tools":[]})).is_empty());
        assert_eq!(
            setup_tools(&json!({"tools":[{"tool":"opencode","installed":true}]})),
            vec!["opencode"]
        );
        assert!(setup_tools(&json!({"tools":[{"tool":"opencode","installed":false}]})).is_empty());
    }

    /// Ron's review of #1419 (2026-10-15): the install block, and Add's
    /// absence, only while no tool this shell launches is installed.
    #[test]
    fn the_install_block_shows_only_when_no_tool_is_installed() {
        let none = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":false},{"tool":"opencode","installed":false}]});
        let one = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":true},{"tool":"opencode","installed":false}]});
        let only_opencode = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":false},{"tool":"opencode","installed":true}]});
        assert_eq!(install_block(&none).len(), 3);
        assert!(install_block(&one).is_empty());
        assert!(install_block(&only_opencode).is_empty());
        assert!(install_block(&json!({})).is_empty());
    }

    /// The caption and Install key on the inventory. OpenCode has no
    /// subscription, so it never offers Reconnect (Ron, #1419).
    #[test]
    fn an_installed_opencode_account_is_not_reported_missing() {
        let installed = json!({"tools":[{"tool":"claude","installed":true},{"tool":"opencode","installed":true}]});
        let absent = json!({"tools":[{"tool":"claude","installed":true},{"tool":"opencode","installed":false}]});
        let account = json!({"tool":"opencode","connection":"api_key"});
        assert!(!tool_missing(&installed, "opencode"));
        assert!(tool_missing(&absent, "opencode"));
        assert!(!tool_missing(&json!({}), "opencode"));
        assert!(can_launch(&installed, "opencode"));
        assert!(!can_launch(&absent, "opencode"));
        assert!(!offers_reconnect(
            &installed,
            &json!({"tool":"opencode","connection":"subscription"})
        ));
        assert_eq!(
            launch_accounts(&json!({"tools":installed["tools"],"accounts":[account.clone()]})),
            vec![account.clone()]
        );
        assert!(launch_accounts(&json!({"tools":absent["tools"],"accounts":[account]})).is_empty());
    }

    /// A missing tool's account keeps its row and caption, with the tool
    /// named; Reconnect is hidden, not disabled.
    #[test]
    fn a_missing_tool_hides_reconnect_and_names_the_tool() {
        let snapshot = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":true}]});
        let claude = json!({"tool":"claude","connection":"subscription"});
        let codex = json!({"tool":"codex","connection":"subscription"});
        assert!(!offers_reconnect(&snapshot, &claude));
        assert!(offers_reconnect(&snapshot, &codex));
        assert!(!offers_reconnect(
            &snapshot,
            &json!({"tool":"codex","connection":"api_key"})
        ));
        assert_eq!(
            tool_unavailable("claude"),
            "Claude Code wasn't found on this computer."
        );
    }

    #[test]
    fn a_new_daemon_is_drawn_from_its_first_revision() {
        let seen = std::cell::Cell::new(0);
        assert!(accepts_revision(&seen, 7));
        // A slow answer from before a change on the same daemon.
        assert!(!accepts_revision(&seen, 6));
        // The switch-over: the host daemon counts from its own start.
        backend_changed(&seen);
        assert!(
            accepts_revision(&seen, 3),
            "the new daemon's snapshot was ignored"
        );
        assert!(accepts_revision(&seen, 3));
        assert!(!accepts_revision(&seen, 2));
    }

    /// The host daemon installed but stopped: the window is attached over
    /// the host link to a socket nobody answers. The start area still draws
    /// its unavailable line and "Set up", so "Start it" can be reached.
    #[test]
    fn an_unanswered_snapshot_in_a_sandbox_still_offers_set_up() {
        assert!(draws_unreachable("daemon-not-running", true));
        // Other failures, and a window outside a sandbox, draw nothing new.
        assert!(!draws_unreachable("managed-storage-unavailable", true));
        assert!(!draws_unreachable("daemon-not-running", false));
        let snapshot = unreachable_snapshot();
        assert!(crate::service::offers_set_up(&snapshot, true));
        assert!(launch_accounts(&snapshot).is_empty());
        assert_eq!(session_destination(&snapshot), None);
        assert_eq!(
            crate::service::launch_press(&snapshot, true, false, false),
            crate::service::LaunchPress::OfferService
        );
    }
}

#[cfg(test)]
mod red_team_row_tests {
    use super::*;

    fn snap(tools: Value, on_host: Value) -> Value {
        json!({
            "tools": tools,
            "capabilities": {"daemon_on_host": on_host},
            "sign_in_accounts": [
                {"id": "c", "tool": "claude", "connection": "near_ai_signin"},
                {"id": "r", "tool": "red_team", "connection": "near_ai_signin"}
            ],
            "generations": {"red_team": 3}
        })
    }

    #[test]
    fn the_daemons_row_gives_state_and_the_problem_the_person_fixes() {
        let ready = snap(
            json!([{"tool": "red_team", "installed": true}]),
            json!(true),
        );
        assert_eq!(
            red_team_row(&ready, true),
            RedTeamRow::Row {
                installed: true,
                problem_key: None,
                fix_command: None
            }
        );
        let kvm = snap(
            json!([{"tool": "red_team", "installed": false, "host_problem": "kvm_not_accessible",
                    "enable_offered": false, "fix_command": "sudo usermod -aG kvm \"$USER\""}]),
            json!(true),
        );
        assert_eq!(
            red_team_row(&kvm, true),
            RedTeamRow::Row {
                installed: false,
                problem_key: Some("red_team_kvm_access"),
                fix_command: Some("sudo usermod -aG kvm \"$USER\"".into()),
            }
        );
        for (problem, key) in [
            ("no_kvm", "red_team_no_kvm"),
            ("no_user_session", "red_team_no_user_session"),
        ] {
            let row = snap(
                json!([{"tool": "red_team", "installed": false, "host_problem": problem}]),
                json!(true),
            );
            assert_eq!(
                red_team_row(&row, true),
                RedTeamRow::Row {
                    installed: false,
                    problem_key: Some(key),
                    fix_command: None
                }
            );
        }
    }

    #[test]
    fn a_windows_problem_is_not_this_shells_to_explain() {
        assert_eq!(red_team_problem_key("hyper_v_missing"), None);
        assert_eq!(red_team_problem_key("home_edition"), None);
    }

    #[test]
    fn a_sandboxed_daemon_sends_no_row_and_the_row_points_at_the_host_service() {
        let sandboxed = snap(json!([{"tool": "claude", "installed": true}]), json!(false));
        assert_eq!(red_team_row(&sandboxed, true), RedTeamRow::FlatpakHost);
        // Not on aarch64, where there is no Red Team to install.
        assert_eq!(red_team_row(&sandboxed, false), RedTeamRow::Hidden);
    }

    #[test]
    fn a_host_daemon_or_an_older_one_without_a_row_shows_nothing() {
        let host = snap(json!([{"tool": "claude", "installed": true}]), json!(true));
        assert_eq!(red_team_row(&host, true), RedTeamRow::Hidden);
        let older = json!({"tools": [], "capabilities": {}});
        assert_eq!(red_team_row(&older, true), RedTeamRow::Hidden);
    }

    #[test]
    fn start_is_the_sign_in_request_in_the_home_folder() {
        let ready = snap(
            json!([{"tool": "red_team", "installed": true}]),
            json!(true),
        );
        let request = red_team_start_request(&ready, "req-1").unwrap();
        assert_eq!(
            request,
            json!({"request_id": "req-1", "purpose": "coding", "tool": "red_team",
                   "connection": "near_ai_signin", "account_id": "r",
                   "expected_generation": 3, "save_default": false})
        );
        assert!(request.get("cwd").is_none());
        let signed_out = json!({"sign_in_accounts": [], "generations": {}});
        assert!(red_team_start_request(&signed_out, "x").is_none());
    }
}
