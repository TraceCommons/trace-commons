//! Changing the NEAR AI login an account uses (F10, spec
//! `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`,
//! section 2.3): the three entry points in the Connection card and the
//! three-step window, at parity with the native macOS client.
//!
//! **Render, do not decide.** Every sentence is the core's approved copy
//! (`account_change_copy`), and the page for each step comes from the
//! core's `change_page`, the same mapping the macOS shell holds in
//! TCShellCore. The daemon runs the ceremony (`account_login_change`) and
//! every network call; this opens the browser for the two browser legs and
//! asks how they went until they end.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use trace_commons_contributor::account_change_copy::{self as words, ChangeState, PageAction};

use super::App;
use super::style::{self, space};
use crate::model::Status;

/// How often an open browser leg is asked about.
const POLL_SECONDS: u32 = 1;

/// Which entry points a status shows (spec 2.1). Pure, so it is held by
/// tests rather than by a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EntryPoints {
    /// The approved mismatch body, with Sign in.
    pub mismatch: bool,
    /// "Use a different account" on the mismatch block: only while the
    /// refused login is held aside for the change.
    pub mismatch_change: bool,
    /// "Use a different account" in the Connection card.
    pub change: bool,
    /// The payouts line of a wallet-origin account, with "Add a NEAR AI
    /// login".
    pub payouts_line: bool,
}

pub(crate) fn entry_points(status: &Status) -> EntryPoints {
    let held = status.login_change_available && status.login_change_held;
    EntryPoints {
        mismatch: status.login_mismatch || held,
        mismatch_change: held,
        change: status.logged_in
            && status.login_change_available
            && !held
            && !status.payouts_need_login,
        payouts_line: status.login_change_available && status.payouts_need_login,
    }
}

/// The ceremony as this window last saw it.
#[derive(Default)]
pub struct ChangeView {
    state: RefCell<Option<ChangeState>>,
    window: RefCell<Option<(gtk::Window, gtk::Box)>>,
    /// The NEAR account name being typed in step 1 of a wallet-origin
    /// account, kept across the page's redraws.
    wallet_account: RefCell<String>,
    calling: Cell<bool>,
    polling: Cell<bool>,
    start_failed: Cell<bool>,
}

/// The Connection card's block: the mismatch body and its buttons, the
/// change button, and the payouts line. Rebuilt on every status.
pub(crate) fn render_block(app: &Rc<App>, block: &gtk::Box, status: &Status) {
    while let Some(child) = block.first_child() {
        block.remove(&child);
    }
    let shown = entry_points(status);
    let view = &app.account_change;
    let busy = view.calling.get();
    if shown.mismatch {
        style::append_body(block, words::ACCOUNT_LOGIN_MISMATCH_BODY);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let sign_in = gtk::Button::with_label(words::ACCOUNT_LOGIN_MISMATCH_SIGN_IN);
        let a = Rc::clone(app);
        sign_in.connect_clicked(move |_| {
            super::credential::act(&a, crate::copy::CredentialAction::Obtain);
        });
        row.append(&sign_in);
        if shown.mismatch_change {
            row.append(&begin_button(
                app,
                words::ACCOUNT_LOGIN_MISMATCH_CHANGE,
                busy,
            ));
        }
        block.append(&row);
    }
    if shown.change {
        block.append(&begin_button(app, words::ACCOUNT_LOGIN_CHANGE, busy));
    }
    if shown.payouts_line {
        style::append_body(block, words::PAYOUTS_NEEDS_LOGIN_BODY);
        block.append(&begin_button(app, words::PAYOUTS_NEEDS_LOGIN_ADD, busy));
    }
    let anything = shown.mismatch || shown.change || shown.payouts_line;
    if anything && view.start_failed.get() {
        style::append_body(block, words::ACCOUNT_CHANGE_ERROR_UNAVAILABLE);
    }
    block.set_visible(anything);
}

fn begin_button(app: &Rc<App>, label: &str, busy: bool) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_halign(gtk::Align::Start);
    button.set_sensitive(!busy);
    let a = Rc::clone(app);
    button.connect_clicked(move |_| begin(&a));
    button
}

/// Start (or resume) the ceremony and open the window. A login held aside
/// moves into it, so its step 2 is skipped.
fn begin(app: &Rc<App>) {
    let view = &app.account_change;
    if view.calling.replace(true) {
        return;
    }
    view.start_failed.set(false);
    view.wallet_account.replace(String::new());
    app.call(
        "account_login_change",
        serde_json::json!({ "step": "start" }),
        |app, result| {
            app.account_change.calling.set(false);
            match result
                .ok()
                .and_then(|value| serde_json::from_value(value).ok())
            {
                Some(state) => {
                    app.account_change.state.replace(Some(state));
                    present(app);
                }
                None => {
                    app.account_change.start_failed.set(true);
                    app.refresh();
                }
            }
        },
    );
}

fn present(app: &Rc<App>) {
    let view = &app.account_change;
    if view.window.borrow().is_none() {
        let content = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let window = gtk::Window::builder()
            .title(words::ACCOUNT_CHANGE_TITLE)
            .transient_for(&app.window)
            .modal(true)
            .default_width(460)
            .child(&content)
            .build();
        // Closing the window is Cancel: the daemon drops the ceremony and
        // any login held for it.
        let a = Rc::clone(app);
        window.connect_close_request(move |_| {
            cancel(&a, false);
            glib::Propagation::Proceed
        });
        view.window.replace(Some((window, content)));
    }
    redraw(app);
    if let Some((window, _)) = view.window.borrow().as_ref() {
        window.present();
    }
}

/// Draw the page for the current state.
fn redraw(app: &Rc<App>) {
    let view = &app.account_change;
    let Some(state) = view.state.borrow().clone() else {
        return;
    };
    let Some((window, content)) = view.window.borrow().clone() else {
        return;
    };
    let Some(page) = words::change_page(&state) else {
        // Words this state cannot draw: close rather than show a placeholder.
        window.close();
        return;
    };
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    let title = gtk::Label::builder().xalign(0.0).wrap(true).build();
    title.set_text(page.title);
    title.add_css_class("tc-card-title");
    content.append(&title);
    if let Some(heading) = page.heading {
        style::append_body(&content, heading);
    }
    for line in &page.lines {
        style::append_body(&content, line);
    }
    let working = page.busy || view.calling.get();
    if page.asks_wallet_account {
        // The wallet join's own field label.
        let label = trace_commons_contributor::witness_copy::witness_copy()
            .wallet
            .account;
        let entry = gtk::Entry::builder()
            .placeholder_text(label)
            .text(view.wallet_account.borrow().as_str())
            .sensitive(!working)
            .build();
        let a = Rc::clone(app);
        entry.connect_changed(move |entry| {
            a.account_change
                .wallet_account
                .replace(entry.text().to_string());
        });
        content.append(&entry);
    }
    if working {
        content.append(
            &gtk::Spinner::builder()
                .spinning(true)
                .halign(gtk::Align::Start)
                .build(),
        );
    }
    if let Some(error) = &page.error {
        style::append_body(&content, error);
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    buttons.set_halign(gtk::Align::End);
    let cancel_button = gtk::Button::with_label(page.cancel);
    let a = Rc::clone(app);
    cancel_button.connect_clicked(move |_| cancel(&a, true));
    buttons.append(&cancel_button);
    let primary = gtk::Button::with_label(page.primary);
    primary.update_property(&[gtk::accessible::Property::Label(
        &page.primary_accessibility_name,
    )]);
    // Switch moves the account: never the window's default.
    if page.action != PageAction::Confirm {
        primary.add_css_class("suggested-action");
    }
    let wallet_empty = page.asks_wallet_account && view.wallet_account.borrow().trim().is_empty();
    primary.set_sensitive(!working && !wallet_empty);
    let a = Rc::clone(app);
    let action = page.action;
    primary.connect_clicked(move |_| step(&a, action));
    buttons.append(&primary);
    content.append(&buttons);
}

/// The page's primary button.
fn step(app: &Rc<App>, action: PageAction) {
    let view = &app.account_change;
    let Some(state) = view.state.borrow().clone() else {
        return;
    };
    let id = state.change_id.clone();
    let params = match action {
        PageAction::Close => {
            close(app);
            return;
        }
        PageAction::ProveCurrent => {
            let mut params = serde_json::json!({ "step": "prove_current", "change_id": id });
            if state.is_wallet() {
                let account = view.wallet_account.borrow().trim().to_owned();
                if account.is_empty() {
                    return;
                }
                params["wallet_account_id"] = account.into();
            }
            params
        }
        PageAction::SignInNew => serde_json::json!({ "step": "sign_in_new", "change_id": id }),
        PageAction::Confirm => {
            serde_json::json!({ "step": "confirm", "change_id": id, "confirmed": true })
        }
    };
    if view.calling.replace(true) {
        return;
    }
    redraw(app);
    app.call("account_login_change", params, move |app, result| {
        app.account_change.calling.set(false);
        let answer: Option<ChangeState> = result.ok().and_then(|v| serde_json::from_value(v).ok());
        match answer {
            Some(next) => {
                if let Some(url) = next.browser_url.as_deref() {
                    if !super::credential::open_browser(url) {
                        // No browser took it: nothing will come back.
                        cancel(app, true);
                        return;
                    }
                }
                let done = next.step == "done";
                let busy = next.busy;
                app.account_change.state.replace(Some(next));
                if done {
                    close(app);
                    app.refresh();
                    return;
                }
                redraw(app);
                if busy {
                    ensure_poll(app);
                }
            }
            None => reread(app),
        }
    });
}

/// Ask how a browser leg is going until it ends.
fn ensure_poll(app: &Rc<App>) {
    let view = &app.account_change;
    if view.polling.replace(true) {
        return;
    }
    let a = Rc::clone(app);
    glib::timeout_add_seconds_local(POLL_SECONDS, move || {
        let busy = a
            .account_change
            .state
            .borrow()
            .as_ref()
            .is_some_and(|state| state.busy);
        if !busy || a.account_change.window.borrow().is_none() {
            a.account_change.polling.set(false);
            return glib::ControlFlow::Break;
        }
        reread(&a);
        glib::ControlFlow::Continue
    });
}

fn reread(app: &Rc<App>) {
    let Some(id) = app
        .account_change
        .state
        .borrow()
        .as_ref()
        .map(|state| state.change_id.clone())
    else {
        return;
    };
    app.call(
        "account_login_change",
        serde_json::json!({ "step": "status", "change_id": id }),
        |app, result| {
            if let Some(next) = result
                .ok()
                .and_then(|v| serde_json::from_value::<ChangeState>(v).ok())
            {
                app.account_change.state.replace(Some(next));
                redraw(app);
            }
        },
    );
}

/// Cancel: the daemon drops the ceremony and any login held for it.
fn cancel(app: &Rc<App>, close_window: bool) {
    let id = app
        .account_change
        .state
        .borrow()
        .as_ref()
        .map(|state| state.change_id.clone());
    let mut params = serde_json::json!({});
    if let Some(id) = id {
        params["change_id"] = id.into();
    }
    app.call("account_login_change_cancel", params, |app, _| {
        app.refresh()
    });
    app.account_change.state.replace(None);
    if close_window {
        close(app);
    }
}

fn close(app: &Rc<App>) {
    let view = &app.account_change;
    view.state.replace(None);
    view.wallet_account.replace(String::new());
    view.polling.set(false);
    if let Some((window, _)) = view.window.take() {
        window.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(fields: serde_json::Value) -> Status {
        let mut value = serde_json::json!({
            "schema_version": "v", "logged_in": true, "paused": false, "queue_depth": 0,
        });
        if let (Some(value), serde_json::Value::Object(fields)) = (value.as_object_mut(), fields) {
            value.extend(fields);
        }
        serde_json::from_value(value).expect("status")
    }

    #[test]
    fn an_older_daemon_shows_nothing() {
        assert_eq!(
            entry_points(&status(serde_json::json!({}))),
            EntryPoints::default()
        );
    }

    #[test]
    fn the_mismatch_shows_alone_and_offers_the_change_only_with_a_held_login() {
        let refused = entry_points(&status(serde_json::json!({ "login_mismatch": true })));
        assert!(refused.mismatch);
        assert!(!refused.mismatch_change);
        assert!(!refused.change);
        let held = entry_points(&status(serde_json::json!({
            "login_change_available": true, "login_change_held": true,
        })));
        assert!(held.mismatch);
        assert!(held.mismatch_change);
        assert!(!held.change, "the mismatch block offers it");
    }

    #[test]
    fn the_change_button_and_the_payouts_line_follow_the_status() {
        let near_ai = entry_points(&status(
            serde_json::json!({ "login_change_available": true }),
        ));
        assert!(near_ai.change);
        assert!(!near_ai.payouts_line);
        let wallet = entry_points(&status(serde_json::json!({
            "login_change_available": true, "payouts_need_login": true,
        })));
        assert!(wallet.payouts_line);
        assert!(!wallet.change, "the payouts line offers it");
        let signed_out = entry_points(&status(serde_json::json!({
            "logged_in": false, "login_change_available": true,
        })));
        assert!(!signed_out.change);
        let unavailable = entry_points(&status(serde_json::json!({ "payouts_need_login": true })));
        assert!(!unavailable.payouts_line);
    }
}
