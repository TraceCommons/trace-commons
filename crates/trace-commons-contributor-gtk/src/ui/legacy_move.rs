//! Moving an invite identity to a NEAR AI account.
//!
//! A device that joined with an invite is offered the move while
//! `status.legacy_invite_migration.offered` holds. Two surfaces draw it: a
//! dismissible card in the notices column, and a card in Settings that stays
//! for as long as the daemon makes the offer. Both are one set of controls
//! over one state, so a refusal reads the same on either.
//!
//! **Nothing here authors a sentence.** The offer, the working line, the
//! passkey lines and every refusal come from
//! `trace_commons_contributor::consent_copy`, which macOS reaches across the
//! C ABI. The `legacy_migration_*` strings below are the daemon's wire
//! labels, which are not words anybody reads.
//!
//! **The passkey step comes first.** The move provisions the account with the
//! NEAR AI session the daemon keeps. With none, the daemon refuses with
//! `legacy_migration_no_near_ai_session`; this shell then runs the same
//! browser sign-in the model-calls screen runs (`near_ai_credential_*`), waits
//! for it, and asks for the move again, once, without a second press. A
//! sign-in that does not finish, or is cancelled, moves nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{App, credential, style};
use crate::copy;

/// The daemon's refusal for a move that had no session to provision with.
const LABEL_NO_SESSION: &str = "legacy_migration_no_near_ai_session";
/// The daemon could not tell which invite this device joined with.
const LABEL_INVITE_NEEDED: &str = "legacy_migration_invite_needed";
/// Every label a move can end with starts here; anything else is a failure
/// to ask at all.
const LABEL_PREFIX: &str = "legacy_migration_";
/// The ceremony's waiting word (`nearai_credential::ceremony`).
const ATTEMPT_WAITING: &str = "waiting_for_browser";
/// How long the move waits on the browser before it says the step did not
/// finish. The same ten minutes macOS waits.
const PASSKEY_LIMIT_SECONDS: u32 = 600;

/// What the shell does with a move that came back refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// Ask for the invite link, then let the next press send it.
    InviteNeeded,
    /// The core's sentence for this label.
    Refused(&'static str),
    /// Not a migration label: the move was never asked or never answered.
    Unstarted,
}

fn classify(label: &str) -> Failure {
    if !label.starts_with(LABEL_PREFIX) {
        return Failure::Unstarted;
    }
    if label == LABEL_INVITE_NEEDED {
        return Failure::InviteNeeded;
    }
    Failure::Refused(trace_commons_contributor::consent_copy::legacy_migration_refusal_line(label))
}

/// Where a sign-in attempt stands, from `near_ai_credential_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Poll {
    SignedIn,
    Waiting,
    Ended,
}

/// A kept session is signed in. An attempt still waiting on the browser, or a
/// poll that could not be read, waits. Any other attempt word -- cancelled,
/// failed, finished without a session, or one this build does not know -- is
/// over.
fn poll_verdict(status: Option<&serde_json::Value>) -> Poll {
    let Some(status) = status else {
        return Poll::Waiting;
    };
    let session = status
        .get("session_state")
        .and_then(serde_json::Value::as_str);
    if session
        == Some(trace_commons_contributor::daemon::nearai_credential::LABEL_CREDENTIAL_PRESENT)
    {
        return Poll::SignedIn;
    }
    match status
        .get("attempt_status")
        .and_then(serde_json::Value::as_str)
    {
        None | Some(ATTEMPT_WAITING) => Poll::Waiting,
        Some(_) => Poll::Ended,
    }
}

/// What the controls show, decided from the state alone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    invite_field: bool,
    waiting_line: bool,
    cancel: bool,
    working_line: bool,
    move_button: bool,
    alert: Option<Alert>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alert {
    Refusal(&'static str),
    PasskeyFailed,
    StartFailed,
}

#[derive(Debug, Default, Clone)]
struct Inner {
    busy: bool,
    awaiting_passkey: bool,
    invite_needed: bool,
    start_failed: bool,
    passkey_failed: bool,
    refusal: Option<&'static str>,
    invite: String,
}

impl Inner {
    /// Move is hidden, not disabled, while the daemon has asked for the
    /// invite and none has been pasted: pressing it could only be refused the
    /// same way again.
    fn can_move(&self) -> bool {
        !self.invite_needed || !self.invite.trim().is_empty()
    }

    fn shown(&self) -> Shown {
        let alert = if let Some(line) = self.refusal {
            Some(Alert::Refusal(line))
        } else if self.passkey_failed {
            Some(Alert::PasskeyFailed)
        } else if self.start_failed {
            Some(Alert::StartFailed)
        } else {
            None
        };
        Shown {
            invite_field: self.invite_needed,
            waiting_line: self.awaiting_passkey,
            cancel: self.awaiting_passkey,
            working_line: self.busy && !self.awaiting_passkey,
            move_button: !self.busy && self.can_move(),
            alert,
        }
    }
}

/// The state both surfaces draw.
#[derive(Default)]
pub struct MoveState {
    inner: RefCell<Inner>,
    /// Bumped by every press and every cancel, so a poll or an answer that
    /// belongs to an earlier press knows it is stale.
    generation: Cell<u64>,
    /// The sign-in this move started, so Cancel can name it.
    attempt: RefCell<Option<String>>,
}

impl MoveState {
    /// Start a move: `None` when one is already running, else the invite to
    /// send (only once the daemon asked for one).
    fn begin(&self) -> Option<(u64, Option<String>)> {
        let mut inner = self.inner.borrow_mut();
        if inner.busy {
            return None;
        }
        let sent = (inner.invite_needed && !inner.invite.trim().is_empty())
            .then(|| inner.invite.trim().to_owned());
        inner.busy = true;
        inner.refusal = None;
        inner.start_failed = false;
        inner.passkey_failed = false;
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        Some((generation, sent))
    }

    fn stale(&self, generation: u64) -> bool {
        self.generation.get() != generation
    }

    fn edit(&self, change: impl FnOnce(&mut Inner)) {
        change(&mut self.inner.borrow_mut());
    }

    fn snapshot(&self) -> Inner {
        self.inner.borrow().clone()
    }
}

// ---------------------------------------------------------------------------
// The card's local dismissal
// ---------------------------------------------------------------------------

/// A digest of the tenant, so a different invite identity enrolled later is
/// offered the move afresh and the file holds no identifier.
fn dismissal_key(tenant_id: &str) -> Option<String> {
    if tenant_id.is_empty() {
        return None;
    }
    let mut checksum = glib::Checksum::new(glib::ChecksumType::Sha256)?;
    checksum.update(tenant_id.as_bytes());
    checksum.string().map(|digest| digest.to_string())
}

/// One file listing the digests whose card was put away. A local marker: no
/// daemon record fits (`acknowledge_legacy_invite_migration` is the notice
/// after a move) and the card is this shell's alone.
fn dismissal_file() -> Option<std::path::PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("trace-commons").join("legacy-move-dismissed.json"))
}

fn read_dismissed(path: &std::path::Path) -> Vec<String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn is_dismissed(path: Option<&std::path::Path>, tenant_id: &str) -> bool {
    let (Some(path), Some(key)) = (path, dismissal_key(tenant_id)) else {
        return false;
    };
    read_dismissed(path).contains(&key)
}

fn record_dismissal(path: Option<&std::path::Path>, tenant_id: &str) {
    let (Some(path), Some(key)) = (path, dismissal_key(tenant_id)) else {
        return;
    };
    let mut keys = read_dismissed(path);
    if keys.contains(&key) {
        return;
    }
    keys.push(key);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&keys) {
        let _ = std::fs::write(path, bytes);
    }
}

// ---------------------------------------------------------------------------
// The controls
// ---------------------------------------------------------------------------

/// One set of widgets over [`MoveState`]. Built once and refilled, so a
/// refresh -- which runs on every daemon event -- never replaces a
/// half-pasted invite.
pub struct Controls {
    pub root: gtk::Box,
    invite_prompt: gtk::Label,
    invite: gtk::Entry,
    waiting: gtk::Label,
    working: gtk::Label,
    alert: gtk::Label,
    move_button: gtk::Button,
    cancel: gtk::Button,
    dismiss: Option<gtk::Button>,
}

impl Controls {
    /// `dismissible` is the notices card; Settings has no dismiss, because
    /// the offer stays there for as long as the daemon makes it.
    pub fn new(dismissible: bool) -> Self {
        let offer = copy::legacy_migration_offer();
        let root = style::card(gtk::Orientation::Vertical, style::space::S);
        root.set_visible(false);
        if dismissible {
            root.remove_css_class("tc-card");
            root.add_css_class("tc-banner");
        }
        let title = style::body(offer.title);
        title.remove_css_class("tc-body");
        title.add_css_class("tc-card-title");
        root.append(&title);
        style::append_body(&root, offer.body);
        if dismissible {
            style::append_body(&root, offer.later);
        }

        let invite = gtk::Entry::builder()
            .placeholder_text(offer.invite_prompt)
            .visible(false)
            .build();
        invite.update_property(&[gtk::accessible::Property::Label(offer.invite_prompt)]);
        // The prompt is the field's label; hidden with it.
        let invite_prompt = style::body(offer.invite_prompt);
        invite_prompt.set_visible(false);
        root.append(&invite_prompt);
        root.append(&invite);

        let waiting = style::body(offer.passkey_waiting);
        let working = style::body(offer.working);
        waiting.set_visible(false);
        working.set_visible(false);
        root.append(&waiting);
        root.append(&working);

        // The action first, the dismiss after it as a flat button.
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, style::space::M);
        let move_button = gtk::Button::with_label(offer.action);
        move_button.add_css_class("suggested-action");
        move_button.add_css_class("pill");
        let cancel = gtk::Button::with_label(offer.passkey_cancel);
        cancel.add_css_class("flat");
        cancel.set_visible(false);
        buttons.append(&move_button);
        buttons.append(&cancel);
        let dismiss = dismissible.then(|| {
            let dismiss = gtk::Button::with_label(offer.dismiss);
            dismiss.add_css_class("flat");
            buttons.append(&dismiss);
            dismiss
        });
        root.append(&buttons);

        let alert = style::body("");
        alert.add_css_class("tc-refused");
        alert.set_visible(false);
        root.append(&alert);

        Self {
            root,
            invite_prompt,
            invite,
            waiting,
            working,
            alert,
            move_button,
            cancel,
            dismiss,
        }
    }

    fn paint(&self, inner: &Inner) {
        let offer = copy::legacy_migration_offer();
        let shown = inner.shown();
        self.invite.set_visible(shown.invite_field);
        self.invite_prompt.set_visible(shown.invite_field);
        self.invite.set_sensitive(!inner.busy);
        if self.invite.text().as_str() != inner.invite {
            self.invite.set_text(&inner.invite);
        }
        self.waiting.set_visible(shown.waiting_line);
        self.working.set_visible(shown.working_line);
        self.cancel.set_visible(shown.cancel);
        self.move_button.set_visible(shown.move_button);
        if let Some(dismiss) = &self.dismiss {
            dismiss.set_sensitive(!inner.busy);
        }
        match shown.alert {
            Some(alert) => {
                self.alert.set_label(match alert {
                    Alert::Refusal(line) => line,
                    Alert::PasskeyFailed => offer.passkey_failed,
                    Alert::StartFailed => offer.start_failed,
                });
                self.alert.set_visible(true);
            }
            None => self.alert.set_visible(false),
        }
    }
}

/// Whether the daemon offers the move, and for which tenant.
fn offered(status: &crate::model::Status) -> bool {
    status
        .legacy_invite_migration
        .get("offered")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Draw both surfaces from `status`. Called from the status handler, before
/// `app.status` holds it. Returns whether the notices card is showing, which
/// the notices column needs to decide whether to be.
pub fn repaint_for(app: &Rc<App>, status: &crate::model::Status) -> bool {
    let offered = offered(status);
    let dismissed = status
        .tenant_id
        .as_deref()
        .is_some_and(|tenant| is_dismissed(dismissal_file().as_deref(), tenant));
    let inner = app.legacy_move.snapshot();
    app.settings.legacy_move.root.set_visible(offered);
    app.settings.legacy_move.paint(&inner);
    let card_shown = offered && !dismissed;
    app.legacy_card.root.set_visible(card_shown);
    app.legacy_card.paint(&inner);
    card_shown
}

/// [`repaint_for`] on the status already held.
fn repaint(app: &Rc<App>) {
    let status = app.status.borrow().clone();
    if let Some(status) = status {
        repaint_for(app, &status);
    }
}

pub fn wire(app: &Rc<App>) {
    for controls in [&app.settings.legacy_move, &app.legacy_card] {
        let weak = Rc::downgrade(app);
        controls.move_button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(app) = weak.upgrade() {
                    migrate(&app);
                }
            }
        });
        controls.invite.connect_changed({
            let weak = weak.clone();
            move |entry| {
                let Some(app) = weak.upgrade() else { return };
                let text = entry.text().to_string();
                if app.legacy_move.snapshot().invite == text {
                    return;
                }
                app.legacy_move.edit(|inner| inner.invite = text);
                repaint(&app);
            }
        });
        controls.cancel.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(app) = weak.upgrade() {
                    cancel_passkey(&app);
                }
            }
        });
    }
    if let Some(dismiss) = &app.legacy_card.dismiss {
        let weak = Rc::downgrade(app);
        dismiss.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else { return };
            let tenant = app
                .status
                .borrow()
                .as_ref()
                .and_then(|status| status.tenant_id.clone());
            if let Some(tenant) = tenant {
                record_dismissal(dismissal_file().as_deref(), &tenant);
            }
            repaint(&app);
            // The notices column hides with its last card.
            app.refresh();
        });
    }
}

// ---------------------------------------------------------------------------
// The move
// ---------------------------------------------------------------------------

/// Move: `legacy_invite_migrate`, which the contributor chose. A refusal
/// changes nothing and is said in the core's words.
fn migrate(app: &Rc<App>) {
    let Some((generation, sent)) = app.legacy_move.begin() else {
        return;
    };
    repaint(app);
    ask(app, sent, generation, true);
}

fn ask(app: &Rc<App>, sent: Option<String>, generation: u64, passkey_allowed: bool) {
    let params = serde_json::json!({ "invite": sent });
    let retry = sent;
    app.call_with_failure("legacy_invite_migrate", params, move |app, result| {
        if app.legacy_move.stale(generation) {
            return;
        }
        match result {
            Ok(_) => {
                app.legacy_move.edit(|inner| {
                    inner.busy = false;
                    inner.invite_needed = false;
                    inner.invite.clear();
                });
                finish(app);
            }
            Err(failure) if passkey_allowed && failure.label == LABEL_NO_SESSION => {
                passkey_step(app, retry, generation);
            }
            Err(failure) => {
                let class = classify(&failure.label);
                app.legacy_move.edit(|inner| {
                    inner.busy = false;
                    match class {
                        Failure::InviteNeeded => {
                            inner.invite_needed = true;
                            inner.refusal = Some(
                                trace_commons_contributor::consent_copy::legacy_migration_refusal_line(
                                    LABEL_INVITE_NEEDED,
                                ),
                            );
                        }
                        Failure::Refused(line) => inner.refusal = Some(line),
                        Failure::Unstarted => inner.start_failed = true,
                    }
                });
                finish(app);
            }
        }
    });
}

/// Re-read status, which drops the offer after a move and brings its notice,
/// and draw what the state now says.
fn finish(app: &Rc<App>) {
    repaint(app);
    app.refresh();
}

/// The passkey step: the first run's browser sign-in, then the move again.
fn passkey_step(app: &Rc<App>, sent: Option<String>, generation: u64) {
    app.legacy_move.edit(|inner| inner.awaiting_passkey = true);
    repaint(app);
    app.call(
        "near_ai_credential_status",
        serde_json::json!({}),
        move |app, result| {
            if app.legacy_move.stale(generation) {
                return;
            }
            if poll_verdict(result.ok().as_ref()) == Poll::SignedIn {
                passkey_done(app, sent, generation);
            } else {
                start_sign_in(app, sent, generation);
            }
        },
    );
}

/// The sign-in finished: the move again, once.
fn passkey_done(app: &Rc<App>, sent: Option<String>, generation: u64) {
    app.legacy_move.edit(|inner| inner.awaiting_passkey = false);
    credential::refresh(app);
    repaint(app);
    ask(app, sent, generation, false);
}

/// The sign-in did not finish. Nothing was moved and the offer stays.
fn passkey_failed(app: &Rc<App>) {
    app.legacy_move.attempt.replace(None);
    app.legacy_move.edit(|inner| {
        inner.awaiting_passkey = false;
        inner.busy = false;
        inner.passkey_failed = true;
    });
    credential::refresh(app);
    finish(app);
}

fn cancel_attempt(app: &Rc<App>, attempt: Option<String>) {
    // Omitted rather than empty when no attempt is known: the daemon then
    // stops whatever sign-in it is running, and an empty id would be refused.
    let params = match attempt {
        Some(id) => serde_json::json!({ "attempt_id": id }),
        None => serde_json::json!({}),
    };
    app.call("near_ai_credential_cancel", params, |app, _| {
        credential::refresh(app);
    });
}

fn start_sign_in(app: &Rc<App>, sent: Option<String>, generation: u64) {
    // No provider named: the daemon's default, as the first run asks for it.
    app.call(
        "near_ai_credential_start",
        serde_json::json!({}),
        move |app, result| {
            let started = result.ok().and_then(|value| {
                let id = value
                    .get("attempt_id")
                    .and_then(serde_json::Value::as_str)?
                    .to_owned();
                let url = value
                    .get("browser_url")
                    .and_then(serde_json::Value::as_str)
                    .filter(|url| !url.is_empty())?
                    .to_owned();
                Some((id, url))
            });
            let Some((id, url)) = started else {
                if !app.legacy_move.stale(generation) {
                    passkey_failed(app);
                }
                return;
            };
            // Cancelled before the browser opened: end the attempt and open
            // nothing.
            if app.legacy_move.stale(generation) {
                cancel_attempt(app, Some(id));
                return;
            }
            if !credential::open_browser(&url) {
                cancel_attempt(app, Some(id));
                passkey_failed(app);
                return;
            }
            app.legacy_move.attempt.replace(Some(id.clone()));
            poll_sign_in(app, sent, generation, id);
        },
    );
}

fn poll_sign_in(app: &Rc<App>, sent: Option<String>, generation: u64, attempt: String) {
    let app = Rc::clone(app);
    let elapsed = Rc::new(Cell::new(0u32));
    let in_flight = Rc::new(Cell::new(false));
    let over = Rc::new(Cell::new(false));
    glib::timeout_add_seconds_local(1, move || {
        if over.get() || app.legacy_move.stale(generation) {
            return glib::ControlFlow::Break;
        }
        elapsed.set(elapsed.get() + 1);
        if elapsed.get() >= PASSKEY_LIMIT_SECONDS {
            over.set(true);
            cancel_attempt(&app, Some(attempt.clone()));
            passkey_failed(&app);
            return glib::ControlFlow::Break;
        }
        if in_flight.replace(true) {
            return glib::ControlFlow::Continue;
        }
        let (over, in_flight, sent) = (over.clone(), in_flight.clone(), sent.clone());
        app.call(
            "near_ai_credential_status",
            serde_json::json!({ "attempt_id": attempt }),
            move |app, result| {
                in_flight.set(false);
                if over.get() || app.legacy_move.stale(generation) {
                    return;
                }
                match poll_verdict(result.ok().as_ref()) {
                    Poll::Waiting => {}
                    Poll::SignedIn => {
                        over.set(true);
                        app.legacy_move.attempt.replace(None);
                        passkey_done(app, sent, generation);
                    }
                    Poll::Ended => {
                        over.set(true);
                        passkey_failed(app);
                    }
                }
            },
        );
        glib::ControlFlow::Continue
    });
}

/// Cancel while waiting on the passkey: stop waiting and end the browser
/// step. Nothing is moved, and the offer stays.
fn cancel_passkey(app: &Rc<App>) {
    let state = &app.legacy_move;
    if !state.snapshot().awaiting_passkey {
        return;
    }
    state.generation.set(state.generation.get() + 1);
    let attempt = state.attempt.replace(None);
    state.edit(|inner| {
        inner.awaiting_passkey = false;
        inner.busy = false;
        inner.passkey_failed = false;
    });
    cancel_attempt(app, attempt);
    finish(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SOURCE: &str = include_str!("legacy_move.rs");

    fn code() -> &'static str {
        SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
    }

    #[test]
    fn a_fresh_offer_shows_move_and_nothing_else() {
        let shown = Inner::default().shown();
        assert!(shown.move_button);
        assert!(!shown.invite_field && !shown.waiting_line && !shown.cancel);
        assert!(!shown.working_line);
        assert_eq!(shown.alert, None);
    }

    #[test]
    fn move_is_hidden_until_an_asked_for_invite_is_pasted() {
        let mut inner = Inner {
            invite_needed: true,
            ..Inner::default()
        };
        let shown = inner.shown();
        assert!(shown.invite_field);
        assert!(!shown.move_button);
        inner.invite = "   ".into();
        assert!(!inner.shown().move_button);
        inner.invite = " https://example.test/invite/abc ".into();
        assert!(inner.shown().move_button);
    }

    #[test]
    fn the_passkey_wait_shows_cancel_and_its_own_line_not_the_working_line() {
        let shown = Inner {
            busy: true,
            awaiting_passkey: true,
            ..Inner::default()
        }
        .shown();
        assert!(shown.waiting_line && shown.cancel);
        assert!(!shown.working_line && !shown.move_button);
    }

    #[test]
    fn a_busy_move_shows_the_working_line_and_no_button() {
        let shown = Inner {
            busy: true,
            ..Inner::default()
        }
        .shown();
        assert!(shown.working_line);
        assert!(!shown.move_button && !shown.cancel);
    }

    #[test]
    fn a_refusal_outranks_the_passkey_and_start_lines() {
        let inner = Inner {
            refusal: Some("line"),
            passkey_failed: true,
            start_failed: true,
            ..Inner::default()
        };
        assert_eq!(inner.shown().alert, Some(Alert::Refusal("line")));
        let inner = Inner {
            passkey_failed: true,
            start_failed: true,
            ..Inner::default()
        };
        assert_eq!(inner.shown().alert, Some(Alert::PasskeyFailed));
    }

    #[test]
    fn begin_refuses_a_second_press_and_sends_the_invite_only_once_asked() {
        let state = MoveState::default();
        state.edit(|inner| inner.invite = "pasted".into());
        let (first, sent) = state.begin().expect("the first press starts");
        assert_eq!(sent, None, "an invite nobody asked for is not sent");
        assert!(state.begin().is_none(), "a second press while busy");
        state.edit(|inner| {
            inner.busy = false;
            inner.invite_needed = true;
            inner.invite = "  pasted  ".into();
        });
        let (second, sent) = state.begin().expect("the next press starts");
        assert_eq!(sent.as_deref(), Some("pasted"));
        assert!(second > first);
        assert!(state.stale(first), "an earlier press's answer is stale");
        assert!(!state.stale(second));
    }

    #[test]
    fn begin_clears_the_last_outcome() {
        let state = MoveState::default();
        state.edit(|inner| {
            inner.refusal = Some("line");
            inner.start_failed = true;
            inner.passkey_failed = true;
        });
        state.begin().expect("starts");
        let inner = state.snapshot();
        assert!(inner.refusal.is_none() && !inner.start_failed && !inner.passkey_failed);
    }

    #[test]
    fn labels_classify_by_the_daemons_contract() {
        assert_eq!(classify(LABEL_INVITE_NEEDED), Failure::InviteNeeded);
        assert_eq!(
            classify("legacy_migration_tenant_pooled"),
            Failure::Refused(
                trace_commons_contributor::consent_copy::legacy_migration_refusal_line(
                    "legacy_migration_tenant_pooled"
                )
            )
        );
        // A label this build does not know still gets the core's fallback.
        assert!(matches!(
            classify("legacy_migration_from_a_later_daemon"),
            Failure::Refused(_)
        ));
        assert_eq!(classify("daemon-not-running"), Failure::Unstarted);
        assert_eq!(classify(""), Failure::Unstarted);
    }

    #[test]
    fn a_kept_session_is_signed_in_a_waiting_or_unread_attempt_waits() {
        let present =
            trace_commons_contributor::daemon::nearai_credential::LABEL_CREDENTIAL_PRESENT;
        assert_eq!(
            poll_verdict(Some(&json!({ "session_state": present }))),
            Poll::SignedIn
        );
        assert_eq!(poll_verdict(None), Poll::Waiting);
        assert_eq!(poll_verdict(Some(&json!({}))), Poll::Waiting);
        assert_eq!(
            poll_verdict(Some(&json!({ "attempt_status": "waiting_for_browser" }))),
            Poll::Waiting
        );
        for over in [
            "cancelled",
            "failed",
            "complete",
            "a_word_from_a_later_daemon",
        ] {
            assert_eq!(
                poll_verdict(Some(&json!({ "attempt_status": over }))),
                Poll::Ended,
                "{over}"
            );
        }
        // A kept session wins over any attempt word.
        assert_eq!(
            poll_verdict(Some(
                &json!({ "session_state": present, "attempt_status": "failed" })
            )),
            Poll::SignedIn
        );
    }

    #[test]
    fn dismissal_is_per_tenant_digest_and_survives_a_reload() {
        let dir = std::env::temp_dir().join(format!(
            "tc-gtk-legacy-move-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let file = dir.join("nested").join("dismissed.json");
        assert!(!is_dismissed(Some(&file), "invite-tenant-a"));
        record_dismissal(Some(&file), "invite-tenant-a");
        record_dismissal(Some(&file), "invite-tenant-a");
        assert!(is_dismissed(Some(&file), "invite-tenant-a"));
        assert!(
            !is_dismissed(Some(&file), "invite-tenant-b"),
            "another identity is offered the move afresh"
        );
        let stored = std::fs::read_to_string(&file).expect("the marker was written");
        assert!(!stored.contains("invite-tenant-a"), "no identifier on disk");
        assert_eq!(read_dismissed(&file).len(), 1, "recorded once");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(dismissal_key("").is_none());
        assert!(!is_dismissed(None, "invite-tenant-a"));
    }

    /// This shell spells no sentence of its own.
    #[test]
    fn this_module_authors_no_user_facing_string() {
        let code = code();
        for needed in [
            "copy::legacy_migration_offer()",
            "consent_copy::legacy_migration_refusal_line",
        ] {
            assert!(code.contains(needed), "{needed} stopped being the source");
        }
        for line in code.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for sentence in ["Your invite", "Nothing was", "passkey", "Move to"] {
                assert!(
                    !trimmed.contains(&format!("\"{sentence}")),
                    "a sentence authored here: {trimmed}"
                );
            }
        }
    }
}
