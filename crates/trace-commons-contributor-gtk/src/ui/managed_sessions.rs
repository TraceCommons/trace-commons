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

fn action(app: &Rc<App>, method: &str, params: Value, terminal: bool) {
    app.call(method, params, move |app, result| match result {
        Ok(value) if terminal => {
            app.call(
                "managed_terminal_launch",
                json!({"session_id":value["session_id"],"ticket":value["ticket"]}),
                |app, result| {
                    if result.is_err() {
                        app.toast(&text("launch_unknown"));
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
        launch.set_sensitive(snapshot["capabilities"]["terminal_launch"] == true && snapshot["accounts"].as_array().is_some_and(|a| !a.is_empty()));
        controls.append(&launch);
        let add_app = Rc::clone(app);
        let terminal = snapshot["capabilities"]["terminal_launch"] == true;
        controls.append(&button("add", move || add_dialog(&add_app, terminal)));
        root.append(&controls);
        let destination = snapshot["capabilities"]["terminal_destination"].as_str();
        style::append_meta(root, destination.map(|d| text("terminal_scope").replace("{destination}", d)).unwrap_or_else(|| text("terminal_unavailable")));
        for account in snapshot["accounts"].as_array().into_iter().flatten() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            style::append_body(&row, format!("{} · {} · {} · {}", account["label"].as_str().unwrap_or_default(), text(account["tool"].as_str().unwrap_or_default()), text(account["connection"].as_str().unwrap_or_default()), account["auth_state"].as_str().unwrap_or_default()));
            let held = snapshot["sessions"].as_array().into_iter().flatten().any(|s| s["account_id"] == account["id"] && s["state"] != "exited" && s["state"] != "failed");
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let a = Rc::clone(app); let account_copy = account.clone();
            let generation = snapshot["generations"][account["tool"].as_str().unwrap_or_default()].clone();
            controls.append(&button("use_default", move || action(&a, "managed_select", json!({"selection":{"tool":account_copy["tool"],"connection":account_copy["connection"],"account_id":account_copy["id"],"generation":generation},"expected_generation":generation}), false)));
            let a = Rc::clone(app); let account_copy = account.clone();
            controls.append(&button("rename_short", move || rename_dialog(&a, &account_copy)));
            if account["connection"] == "subscription" {
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

fn add_dialog(app: &Rc<App>, terminal: bool) {
    let (window, content) = dialog(app, "save_title");
    let tool = gtk::DropDown::from_strings(&[&text("claude"), &text("codex")]);
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
        a.call("managed_account_add", json!({"tool":(["claude","codex"][tool.selected() as usize]),"connection":kind,"label":label.text().as_str()}), move |app, result| {
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
    let Some(accounts) = snapshot["accounts"].as_array() else {
        return;
    };
    let accounts = accounts.clone();
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
