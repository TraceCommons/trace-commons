//! Account and session actions use the same daemon service as the NEAR AI CLI.
use super::{App, style};
use adw::prelude::*;
use serde_json::{Value, json};
use std::rc::Rc;

fn text(key: &str) -> String {
    trace_commons_contributor::managed::copy::copy()[key]
        .as_str()
        .unwrap_or_default()
        .into()
}
fn can_launch(snapshot: &Value, tool: &str) -> bool {
    ["claude", "codex"].contains(&tool)
        && snapshot["tools"].as_array().is_none_or(|tools| {
            tools
                .iter()
                .any(|t| t["tool"] == tool && t["installed"] == true)
        })
}
fn setup_tools(snapshot: &Value) -> Vec<&'static str> {
    ["claude", "codex"]
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
        .filter(|t| ["claude", "codex"].contains(&t["tool"].as_str().unwrap_or_default()))
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
    let glyph = gtk::Button::from_icon_name("view-refresh-symbolic");
    glyph.add_css_class("flat");
    glyph.set_tooltip_text(Some(&text("refresh")));
    glyph.update_property(&[gtk::accessible::Property::Label(&text("refresh"))]);
    let app = Rc::clone(app);
    glyph.connect_clicked(move |_| refresh(&app));
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

/// The line for a refused terminal launch. A chosen terminal that was not
/// found has its own, naming it and pointing at Settings; anything else is
/// the generic launch line.
fn launch_refusal(label: &str, destination: Option<&str>) -> String {
    if label == "managed-terminal-not-found" {
        let terminal = destination
            .map(str::to_owned)
            .unwrap_or_else(|| text("terminal"));
        text("terminal_not_found").replace("{terminal}", &terminal)
    } else {
        text("launch_unknown")
    }
}

fn action(app: &Rc<App>, method: &str, params: Value, terminal: bool) {
    // The terminal the launch opens in, for the not-found line.
    let destination = app.private_inference.managed_destination.borrow().clone();
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
        Err(_) => app.toast(&text("action_failed")),
    });
}

pub fn refresh(app: &Rc<App>) {
    app.call("managed_snapshot", json!({}), |app, result| {
        let Ok(snapshot) = result else { return; };
        let revision = snapshot["revision"].as_u64().unwrap_or_default();
        if revision < app.private_inference.managed_revision.get() { return; }
        app.private_inference.managed_revision.set(revision);
        let root = &app.private_inference.managed;
        clear(root);
        root.append(&style::section(&text("title")));
        style::append_body(root, text("description"));
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let launch_app = Rc::clone(app);
        let launch_snapshot = snapshot.clone();
        let launch = button("launch", move || launch_dialog(&launch_app, &launch_snapshot));
        launch.set_sensitive(snapshot["capabilities"]["terminal_launch"] == true && !launch_accounts(&snapshot).is_empty());
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
        let destination = snapshot["capabilities"]["terminal_destination"].as_str();
        *app.private_inference.managed_destination.borrow_mut() = destination.map(str::to_owned);
        style::append_meta(root, destination.map(|d| text("terminal_scope").replace("{destination}", d)).unwrap_or_else(|| text("terminal_unavailable")));
        for account in snapshot["accounts"].as_array().into_iter().flatten() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            style::append_body(&row, format!("{} · {} · {} · {}", account["label"].as_str().unwrap_or_default(), text(account["tool"].as_str().unwrap_or_default()), text(account["connection"].as_str().unwrap_or_default()), account["auth_state"].as_str().unwrap_or_default()));
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
                style::append_body(root, format!("{project} · {} · {} · {} · {}", text(session["tool"].as_str().unwrap_or_default()), session["account_label"].as_str().unwrap_or_default(), text(session["connection"].as_str().unwrap_or_default()), session["state"].as_str().unwrap_or_default()));
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
    let labels: Vec<_> = tools.iter().map(|id| text(id)).collect();
    let tool = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let connection =
        gtk::DropDown::from_strings(&[&text("subscription"), &text("api_key"), &text("near_ai")]);
    let label = gtk::Entry::builder()
        .placeholder_text(text("label_placeholder"))
        .build();
    let key = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    key.set_visible(false);
    let k = key.clone();
    connection.connect_selected_notify(move |c| k.set_visible(c.selected() != 0));
    content.append(&tool);
    content.append(&connection);
    content.append(&label);
    content.append(&key);
    style::append_body(&content, text("login_description"));
    style::append_meta(&content, text("key_storage"));
    let a = Rc::clone(app);
    let w = window.clone();
    content.append(&button("save", move || {
        let subscription = connection.selected() == 0;
        if label.text().trim().is_empty() || (subscription && !terminal) { a.toast(&text("terminal_unavailable")); return; }
        let secret = key.text().to_string();
        if !subscription && secret.is_empty() { return; }
        let kind = ["subscription", "api_key", "near_ai"][connection.selected() as usize];
        a.call("managed_account_add", json!({"tool":(tools[tool.selected() as usize]),"connection":kind,"label":label.text().as_str()}), move |app, result| {
            if let Ok(account) = result {
                if subscription { action(app, "managed_account_reconnect", json!({"account_id":account["id"]}), true); }
                else { action(app, "managed_account_set_key", json!({"account_id":account["id"],"key":secret}), false); }
            } else { app.toast(&text("action_failed")); }
        });
        key.set_text(""); w.close();
    }));
    window.present();
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
    content.append(&picker);
    let folder = gtk::Entry::builder()
        .placeholder_text(text("project_placeholder"))
        .build();
    content.append(&folder);
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
    style::append_meta(
        &content,
        text("launch_scope").replace(
            "{destination}",
            snapshot["capabilities"]["terminal_destination"]
                .as_str()
                .unwrap_or_default(),
        ),
    );
    let a = Rc::clone(app);
    let generations = snapshot["generations"].clone();
    let w = window.clone();
    content.append(&button("launch_short", move || {
        let Some(account) = accounts.get(picker.selected() as usize) else { return; };
        action(&a, "managed_launch_prepare", json!({"request_id":uuid::Uuid::new_v4(),"purpose":"coding","tool":account["tool"],"connection":account["connection"],"account_id":account["id"],"cwd":folder.text().as_str(),"expected_generation":generations[account["tool"].as_str().unwrap_or_default()],"save_default":false}), true);
        w.close();
    }));
    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn availability_filters_launch_without_removing_saved_accounts() {
        let snapshot = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":true},{"tool":"opencode","installed":true}],
            "accounts":[{"tool":"claude"},{"tool":"codex"}]});
        assert_eq!(setup_tools(&snapshot), vec!["codex"]);
        assert_eq!(launch_accounts(&snapshot), vec![json!({"tool":"codex"})]);
        assert_eq!(snapshot["accounts"].as_array().unwrap().len(), 2);
        // GTK stays on Claude Code and Codex until it launches OpenCode.
        assert!(!can_launch(&snapshot, "opencode"));
    }

    #[test]
    fn older_snapshot_retains_managed_tools_but_empty_inventory_does_not() {
        assert_eq!(setup_tools(&json!({})), vec!["claude", "codex"]);
        assert!(setup_tools(&json!({"tools":[]})).is_empty());
        assert!(setup_tools(&json!({"tools":[{"tool":"opencode","installed":true}]})).is_empty());
    }

    /// Ron's review of #1419 (2026-10-15): the install block, and Add's
    /// absence, only while no tool this shell launches is installed.
    #[test]
    fn the_install_block_shows_only_when_no_tool_is_installed() {
        let none = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":false}]});
        let one = json!({"tools":[{"tool":"claude","installed":false},{"tool":"codex","installed":true}]});
        assert_eq!(install_block(&none).len(), 2);
        assert!(install_block(&one).is_empty());
        assert!(install_block(&json!({})).is_empty());
    }

    /// An OpenCode account is not "missing" because GTK cannot launch it: the
    /// caption and Install key on the inventory, and Launch and Reconnect stay
    /// out for it (Ron, #1419).
    #[test]
    fn an_installed_opencode_account_is_not_reported_missing() {
        let installed = json!({"tools":[{"tool":"claude","installed":true},{"tool":"opencode","installed":true}]});
        let absent = json!({"tools":[{"tool":"claude","installed":true},{"tool":"opencode","installed":false}]});
        let account = json!({"tool":"opencode","connection":"subscription"});
        assert!(!tool_missing(&installed, "opencode"));
        assert!(tool_missing(&absent, "opencode"));
        assert!(!tool_missing(&json!({}), "opencode"));
        assert!(!can_launch(&installed, "opencode"));
        assert!(!offers_reconnect(&installed, &account));
        assert!(
            launch_accounts(&json!({"tools":installed["tools"],"accounts":[account]})).is_empty()
        );
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
}
