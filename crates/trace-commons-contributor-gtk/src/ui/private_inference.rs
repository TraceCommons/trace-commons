//! The model-calls screen: whether this computer answers the model calls
//! made on it, and what the listener is actually doing.
//!
//! Its own destination rather than a card five sections down Settings. The
//! switch here is the one that makes this machine answer calls for whatever
//! else is running on it, and a contributor who has turned it on is owed a
//! place they can look at to see whether it is on right now.
//!
//! **Every sentence is read from `copy`**, which re-exports
//! `trace_commons_contributor::private_inference_copy` -- the same
//! definition the macOS and Windows shells reach across the C ABI. Nothing
//! here is authored in this view. The one word this surface may never say is
//! that the calls are private: turning this on does not make a call private,
//! it moves where the call is answered, and each call still goes on to
//! whoever was configured to answer it. The shared module carries the sweep
//! that enforces that; this view stays out of its way by never retyping a
//! sentence.
//!
//! **The indicator follows the tone, never the switch.** The switch shows
//! what was asked for; the tone -- and only `Clear` in it -- shows what is
//! true. A listener that refused to start leaves the switch on, and a screen
//! that painted a green dot from the boolean would be reporting a service
//! running that is not.
//!
//! **This window carries the whole capability.** GNOME has no tray (see
//! `ui/mod.rs`), so the tray entry beside this is only ever a shortcut in:
//! nothing on this screen may be reachable only from there.
//!
//! # The list leads, and the switch does not
//!
//! A tool is the unit a contributor can decide about. "Answer model calls at
//! all" is a consequence of connecting one, not a question to settle first,
//! so the tools on this computer are the first thing on the screen and the
//! switch below them is the kill switch.
//!
//! # Nothing is written that was not shown first
//!
//! Connecting a tool edits a file this app does not own. `harness_plan`
//! works the edit out and writes nothing; `harness_commit` takes a plan id
//! the daemon minted and nothing else, so this shell cannot construct a
//! write of its own. Between the two there is a dialog with the change in
//! it. A plan is single-use and expires, and a file that moved underneath
//! one is refused -- both are outcomes to report, not errors to retry.
//!
//! An occupied slot rides alongside whatever the outcome says, and is drawn
//! as what it is: a value the contributor put there, left exactly as they
//! had it. It is never an error to clear and never an offer to take over.

use std::rc::Rc;

use adw::prelude::*;
use trace_commons_contributor::harness_state::{HarnessAction, HarnessState, PlanOutcome};

use super::style::{self, Tone, space};
use super::{App, COLUMN_MAX, COLUMN_TIGHTEN};
use crate::copy;

/// What to do once a write has come back confirmed.
///
/// Exists for one caller: the first connect, which has to turn the listener
/// on before there is anywhere for a tool to send its calls. Running it
/// after the daemon's echo rather than beside the request is the same rule
/// the rest of this screen follows -- nothing acts on what was asked for.
type AfterWrite = Box<dyn FnOnce(&Rc<App>)>;

/// The screen's widgets. Built once and refilled, like every other screen
/// in this window: a refresh runs on every daemon event, and rebuilding the
/// switch under a contributor's finger would drop the press.
pub struct PrivateInferenceView {
    pub root: gtk::Box,
    /// What today's calls cost, and what that figure leaves out. Emptied
    /// and refilled on every render for the same reason the list is: an
    /// amount a later read could not measure must go, not linger.
    spend: gtk::Box,
    /// The tools on this computer, one card each. Rebuilt on every render
    /// for the reason the status box is: the list is read each time it is
    /// shown, so a tool that rewrote its own settings file corrects itself
    /// rather than leaving a claim on screen that stopped being true.
    harnesses: gtk::Box,
    /// What was asked for. Insensitive until the daemon's own answer has
    /// arrived, so a press cannot write a value nothing confirmed.
    switch: gtk::Switch,
    /// What the listener is actually doing: one toned sentence, and the
    /// serving line under it when there is a port. Rebuilt on each render.
    status: gtk::Box,
    /// Set while a render is writing the daemon's answer into the switch,
    /// so the signal that fires is not mistaken for a contributor acting.
    filling: std::cell::Cell<bool>,
    /// The last confirmed requested setting. `None` is "not yet heard from",
    /// which is not an off declaration.
    confirmed: std::cell::Cell<Option<bool>>,
    write_pending: std::cell::Cell<bool>,
}

impl Default for PrivateInferenceView {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivateInferenceView {
    pub fn new() -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::XL)
            .margin_top(space::L)
            .margin_bottom(space::XXL)
            .margin_start(space::XL)
            .margin_end(space::XL)
            .build();

        content.append(&style::section(copy::PRIVATE_INFERENCE_TITLE));
        style::append_body(&content, copy::PRIVATE_INFERENCE_SUBTITLE);

        // The list first, and the switch below it. See the module note.
        content.append(&style::section(copy::HARNESSES_TITLE));
        style::append_body(&content, copy::HARNESSES_WHAT);
        let spend = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        content.append(&spend);
        let harnesses = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        content.append(&harnesses);

        let card = style::card(gtk::Orientation::Vertical, space::M);
        style::append_body(&card, copy::PRIVATE_INFERENCE_OFFER_WHAT);
        // The exposure sentence is on this screen, not only in the offer. A
        // contributor who declined and came back months later is making the
        // same decision and is owed the same sentence.
        style::append_body(&card, copy::PRIVATE_INFERENCE_OFFER_EXPOSURE);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let label = gtk::Label::builder()
            .label(copy::PRIVATE_INFERENCE_TOGGLE)
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .build();
        label.add_css_class("tc-body");
        let switch = gtk::Switch::builder().halign(gtk::Align::End).build();
        switch.set_sensitive(false);
        switch.set_valign(gtk::Align::Center);
        switch.update_property(&[gtk::accessible::Property::Label(
            copy::PRIVATE_INFERENCE_TOGGLE,
        )]);
        // How the toggle chord is discovered: the chord and nothing else,
        // spelled by GTK in the contributor's own keyboard and language, so
        // no word of it is authored anywhere in this shell. `None` -- an
        // accelerator GTK would not parse -- leaves no tooltip rather than a
        // wrong one. What the chord then does is `shortcuts`' business and
        // not this file's; pressing it while answering is off opens this
        // screen and writes nothing.
        switch.set_tooltip_text(super::shortcuts::toggle_accel_label().as_deref());
        row.append(&label);
        row.append(&switch);
        card.append(&row);

        // Drawn before anything has been asked, so the screen never opens
        // on a blank space where the state will be. Unreported is what is
        // true at that moment, and the shared table has a sentence for it.
        let status = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        status.append(&tone_row(
            copy::PRIVATE_INFERENCE_STATE_UNKNOWN,
            Tone::Neutral,
        ));
        card.append(&status);
        style::append_caveat(&card, copy::PRIVATE_INFERENCE_APPLIES_AT_ONCE);
        content.append(&card);

        let clamp = adw::Clamp::builder()
            .maximum_size(COLUMN_MAX)
            .tightening_threshold(COLUMN_TIGHTEN)
            .child(&content)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
        root.append(&scroller);

        Self {
            root,
            spend,
            harnesses,
            switch,
            status,
            filling: std::cell::Cell::new(false),
            confirmed: std::cell::Cell::new(None),
            write_pending: std::cell::Cell::new(false),
        }
    }
}

/// A glyph and a sentence, read as one statement.
///
/// A local copy of the shape Settings uses rather than a shared helper: the
/// two screens are free to diverge, and the three lines here are not worth
/// coupling them over.
fn tone_row(label: &str, tone: Tone) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let glyph = gtk::Label::new(Some(tone.glyph()));
    glyph.add_css_class(tone.css());
    glyph.set_valign(gtk::Align::Start);
    row.append(&glyph);
    let text = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .build();
    text.add_css_class("tc-body");
    row.append(&text);
    row.update_property(&[gtk::accessible::Property::Label(label)]);
    row
}

/// The shared tone onto this shell's palette.
///
/// NOT A BRANCH TABLE ON THE STATE: which tone each state reads in is
/// decided once, in `private_inference_copy`, beside the sentence it goes
/// with. This only carries that answer onto `style::Tone`.
///
/// **No catch-all arm, deliberately.** A tone this shell has not been taught
/// must fail to compile rather than fall through to something that reads as
/// working. `indicator_reads_as_working` pins the other half of that rule:
/// `Tone::Clear` comes out of here for `Clear` and for nothing else.
fn indicator_tone(tone: copy::PrivateInferenceTone) -> Tone {
    match tone {
        copy::PrivateInferenceTone::Neutral => Tone::Neutral,
        copy::PrivateInferenceTone::Held => Tone::Held,
        copy::PrivateInferenceTone::Clear => Tone::Clear,
        copy::PrivateInferenceTone::Attention => Tone::Attention,
        copy::PrivateInferenceTone::Refused => Tone::Refused,
    }
}

/// The sentence for one tool's state, read from the shared module.
///
/// A pair with [`harness_tone`], used as one: recovering a tone by reading
/// the sentence would be matching on text.
///
/// **ONE TABLE, ASKED -- NOT A COPY OF IT.** This used to be a `match` here,
/// and Swift and C# each held their own; three copies of one decision. The
/// decision now lives in `private_inference_copy::harness_state_line`, which
/// the other two shells reach as `tc_harness_state_line`. This shell links
/// the crate, so it calls the same function directly. Do not put the arms
/// back.
///
/// `ActivityShared` and `Unknown` answer the EMPTY STRING, which this view
/// draws as no line at all. `ActivityShared` may not borrow the answering
/// sentence -- that sentence says a call from *it* reached this computer,
/// and the pronoun names the row's own tool, which is the one thing this
/// state says is unknown -- and may not borrow the connected one, which
/// would be false. A row that claims nothing is the honest answer to a
/// question the ledger cannot settle.
pub fn harness_line(state: &str) -> &'static str {
    copy::harness_state_line(state)
}

/// The sentence one plan outcome carries, read from the shared module.
///
/// **ONE TABLE, ASKED -- NOT A COPY OF IT.** This used to be an
/// `if outcome == Some(PlanOutcome::Unparseable)` arm reaching for one
/// constant, and macOS and Windows each held the same arm; the four other
/// non-committable outcomes had no sentence at all, so a preview for one of
/// them opened with a title, a path, no changes, no explanation and a way
/// out. The decision now lives in
/// `private_inference_copy::harness_outcome_line`, which the other two shells
/// reach as `tc_harness_outcome_line`. Do not put the arm back.
///
/// `Changes` answers the EMPTY STRING, which this view draws as no row at
/// all: a plan with changes in it shows them, and a sentence above them
/// saying there are changes is this shell narrating its own list. An outcome
/// this build has never heard of answers the same way rather than borrowing
/// the nearest refusal.
pub fn harness_outcome_line(outcome: &str) -> &'static str {
    copy::harness_outcome_line(outcome)
}

/// The tone [`harness_line`]'s sentence is painted in.
///
/// **`Clear` for `Answering` and for nothing else.** Everything else here is
/// a claim about a settings file, or an admission that attribution failed,
/// and neither is evidence that a call was answered. A label this build has
/// never heard of lands in the same arm as `Unknown`, because the unsafe
/// direction on this surface is the working light.
pub fn harness_tone(state: &str) -> Tone {
    match HarnessState::from_label(state) {
        Some(HarnessState::Answering) => Tone::Clear,
        Some(HarnessState::ConnectedNoCalls | HarnessState::ActivityShared) => Tone::Held,
        Some(HarnessState::NotConnected | HarnessState::Unknown) | None => Tone::Neutral,
    }
}

/// How long ago a call arrived, for the shared sentence that says so.
///
/// `None` for an absent or unparseable time, which draws no line at all --
/// the state sentence above it has already said the part that is true.
fn seconds_since(at: Option<&str>) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(at?).ok()?;
    let elapsed = chrono::Utc::now()
        .signed_duration_since(at.with_timezone(&chrono::Utc))
        .num_seconds();
    u64::try_from(elapsed.max(0)).ok()
}

/// Ask the daemon which tools are here, and draw them.
///
/// Read on every refresh rather than latched: a tool can rewrite its own
/// settings file while this window is open, and a latched list would keep
/// claiming the old answer.
pub fn render_harnesses(app: &Rc<App>) {
    app.call("harness_list", serde_json::json!({}), |app, result| {
        let Ok(Ok(list)) = result.map(serde_json::from_value::<crate::model::HarnessList>) else {
            return;
        };
        let view = &app.private_inference;
        while let Some(child) = view.harnesses.first_child() {
            view.harnesses.remove(&child);
        }
        while let Some(child) = view.spend.first_child() {
            view.spend.remove(&child);
        }
        // Assembled on the shared side, and EMPTY WHEN THE FIGURE IS NOT
        // KNOWN -- which draws no row at all rather than a zero. The scope
        // sentence goes with it: on its own it would qualify a figure that
        // is not on screen.
        let spend = copy::harness_spend_line(list.spend.micros());
        if !spend.is_empty() {
            style::append_body(&view.spend, &spend);
            style::append_meta(&view.spend, copy::HARNESSES_SPEND_SCOPE);
        }
        if list.harnesses.is_empty() {
            // An empty list that explains nothing cannot be told apart from
            // a broken one, so the shared sentence says what was looked for.
            style::append_body(&view.harnesses, copy::HARNESSES_NONE_FOUND);
            return;
        }
        for row in &list.harnesses {
            let card = harness_card(app, row);
            app.private_inference.harnesses.append(&card);
        }
    });
}

/// Whether a connect has to put the exposure question first.
///
/// **Deliberately wider than the first-run offer's own predicate**, and the
/// same shape the macOS shell uses so the three do not diverge. `should_offer`
/// asks once and never again; this asks whenever the connect would reopen
/// the listener. Somebody who answered the offer months ago and has since
/// used the kill switch is making the exposure decision afresh, and the
/// sentence about what it exposes has to be in front of them when they do.
///
/// It is not the same question as "has the offer been seen", so it does not
/// read that marker. Accepting still writes it, through the switch's own
/// write path.
fn connect_needs_exposure(listener_on: bool) -> bool {
    !listener_on
}

/// The switch's last confirmed position, which is what the gate turns on.
fn listener_on(app: &Rc<App>) -> bool {
    app.private_inference.confirmed.get().unwrap_or(false)
}

/// One tool.
///
/// The name is the daemon's -- IronWire's `Tool.name` -- and never a word
/// this shell holds. The path and the command are the contributor's own
/// machine talking back at them, so both are shown verbatim and the command
/// is selectable: doing it by hand instead is always available, and this
/// window is the only place a GNOME contributor can find it.
fn harness_card(app: &Rc<App>, row: &crate::model::Harness) -> gtk::Box {
    let card = style::card(gtk::Orientation::Vertical, space::S);

    let name = gtk::Label::builder()
        .label(&row.name)
        .xalign(0.0)
        .wrap(true)
        .build();
    name.add_css_class("tc-card-title");
    card.append(&name);

    // Three of the five states have a sentence and two deliberately do not.
    // An empty sentence is drawn as no row at all, never as a blank one at
    // the working colour.
    //
    // A tool that is not on this computer is not one of those states, and
    // takes the missing-tool sentence INSTEAD. The row is listed rather than
    // hidden -- a tool left out cannot be told apart from one this app was
    // never taught about -- and it may not keep the not-connected sentence,
    // which claims a tool's own settings still send its calls wherever they
    // went before. Before this the two rendered identically, with the connect
    // button simply insensitive and nothing saying why.
    if row.installed {
        let state_line = harness_line(&row.state);
        if !state_line.is_empty() {
            card.append(&tone_row(state_line, harness_tone(&row.state)));
        }
    } else {
        card.append(&tone_row(copy::HARNESS_NOT_INSTALLED, Tone::Neutral));
    }
    // A tool that was already running holds the old setting until it is
    // started again. Said while nothing has arrived, and dropped the moment
    // something has -- at which point it is answered by events.
    if HarnessState::from_label(&row.state) == Some(HarnessState::ConnectedNoCalls) {
        style::append_meta(&card, copy::HARNESS_NEEDS_RESTART);
    }
    // Assembled on the shared side, and empty when there is nothing to
    // report. An empty sentence is drawn as no row at all.
    let last_call = copy::harness_last_call_line(seconds_since(row.last_call_at.as_deref()));
    if !last_call.is_empty() {
        style::append_meta(&card, last_call);
    }
    if let Some(path) = row.config_path.as_deref() {
        style::append_meta(&card, path);
    }
    if !row.connect_command.is_empty() {
        let command = gtk::Label::builder()
            .label(&row.connect_command)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        command.add_css_class("tc-mono");
        card.append(&command);
    }

    // Chosen from the daemon's answer, not from `connected`.
    //
    // `connected` is narrowed to "names OUR destination port"; the daemon
    // derives `can_disconnect` from the broader "names any local proxy", so a
    // line pointing at a stale or foreign port keeps the control that removes
    // it. Picking the action from `!row.connected` handed such a row a Connect
    // button, which the daemon refuses as a no-op -- telling the contributor
    // the file "already says what this would have written" when it names
    // somebody else's port, and offering no way to fix it.
    let connecting = !row.can_disconnect;
    let action = if connecting {
        HarnessAction::Connect
    } else {
        HarnessAction::Disconnect
    };
    let button = gtk::Button::with_label(if connecting {
        copy::HARNESS_CONNECT
    } else {
        copy::HARNESS_DISCONNECT
    });
    button.set_halign(gtk::Align::Start);
    // The daemon's own answer to whether the action may be offered, rather
    // than this shell re-deriving it: a tool that is gone can still be
    // disconnected, because uninstalling it did not remove the line we put
    // in its file.
    //
    // A connect additionally needs somewhere for the calls to go: with
    // nothing answering here there is no destination to write, and the
    // daemon refuses to plan one. That case is not hidden behind a dead
    // control, because it is exactly the case the exposure gate handles --
    // the press turns the listener on first, and the connect is planned
    // against the port that then exists.
    let available = if connecting {
        row.can_connect
    } else {
        row.can_disconnect
    };
    button.set_sensitive(available);
    let app = Rc::clone(app);
    let id = row.id.clone();
    button.connect_clicked(move |_| {
        let id = id.clone();
        if connecting && connect_needs_exposure(listener_on(&app)) {
            present_exposure(&app, &id);
            return;
        }
        plan_harness(&app, &id, action);
    });
    card.append(&button);

    card
}

/// The first connect asks what turning this computer's answering on exposes.
///
/// Asked once, before anything is written, and answered either way: a
/// decline is recorded exactly as an acceptance is, so nobody is asked
/// twice. Accepting turns the listener on through the switch's own write
/// path and then plans the connect, because a connect has nothing to write
/// until there is somewhere for the calls to go.
fn present_exposure(app: &Rc<App>, id: &str) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some(copy::PRIVATE_INFERENCE_OFFER_TITLE),
        Some(copy::PRIVATE_INFERENCE_OFFER_WHAT),
    );
    let body = gtk::Box::new(gtk::Orientation::Vertical, space::M);
    style::append_body(&body, copy::PRIVATE_INFERENCE_OFFER_EXPOSURE);
    style::append_body(&body, copy::PRIVATE_INFERENCE_OFFER_NO_REPOINT);
    // No `OFFER_ASKED_ONCE` here. It promises "this is the only time you will
    // be asked" and that "the switch stays in Settings"; on this surface both
    // are false -- the harness gate re-asks whenever the destination is off,
    // and the switch has moved here from Settings. It stays on the first-run
    // offer, where it is true.
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[
        ("decline", copy::PRIVATE_INFERENCE_OFFER_DECLINE),
        ("accept", copy::PRIVATE_INFERENCE_OFFER_ACCEPT),
    ]);
    dialog.set_close_response("decline");

    let app = Rc::clone(app);
    let id = id.to_string();
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response != "accept" {
            // Still an answer, and remembered as one.
            send(&app, false, None);
            return;
        }
        let id = id.clone();
        send(
            &app,
            true,
            Some(Box::new(move |app: &Rc<App>| {
                plan_harness(app, &id, HarnessAction::Connect);
            })),
        );
    });
    dialog.present();
}

/// Work the edit out. This writes nothing.
fn plan_harness(app: &Rc<App>, id: &str, action: HarnessAction) {
    app.call(
        "harness_plan",
        serde_json::json!({ "id": id, "action": action.label() }),
        |app, result| match result.map(serde_json::from_value::<crate::model::HarnessPlan>) {
            Ok(Ok(plan)) => present_plan(app, &plan),
            _ => app.toast(copy::PRIVATE_INFERENCE_WRITE_UNCONFIRMED),
        },
    );
}

/// Show the change, and commit only what was confirmed.
///
/// The heading names the file, because the file is the contributor's and
/// this app is about to edit it. `changes` are IronWire's own words and are
/// shown verbatim -- a summary of them would be this shell asserting
/// something about a write it did not work out.
///
/// The confirm response exists only where there is something to commit. A
/// plan that has nothing to do, and a file that could not be read at all,
/// each get the way out and nothing else -- and each says which it is,
/// because a refusal is not a file that already said the right thing, and
/// neither of them is an empty sheet.
fn present_plan(app: &Rc<App>, plan: &crate::model::HarnessPlan) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some(copy::HARNESS_PREVIEW_TITLE),
        plan.path.as_deref(),
    );
    let body = gtk::Box::new(gtk::Orientation::Vertical, space::M);
    let outcome = PlanOutcome::from_label(&plan.outcome);
    // Every outcome that writes nothing says why, and which sentence it says
    // is the shared table's decision rather than an arm held here. An empty
    // answer -- `changes`, and an outcome this build has never heard of --
    // draws no row at all.
    // The tone is not a table: it is the one distinction between a plan that
    // found nothing to do and a plan this app refused to carry out. Which
    // sentence appears is decided on the shared side; only how firmly it
    // reads is decided here.
    let outcome_line = harness_outcome_line(&plan.outcome);
    if !outcome_line.is_empty() {
        let tone = if outcome == Some(PlanOutcome::Noop) {
            Tone::Held
        } else {
            Tone::Refused
        };
        body.append(&tone_row(outcome_line, tone));
    }
    for change in &plan.changes {
        style::append_body(&body, change);
    }
    // Whatever the outcome says. A plan can carry changes and occupied
    // slots at once, and the slot is reported as what it is: a value the
    // contributor put there, left alone. Its own value is shown, because a
    // report that hid what it found would not be one.
    if !plan.occupied.is_empty() {
        body.append(&tone_row(copy::HARNESS_SLOT_TAKEN, Tone::Held));
        for slot in &plan.occupied {
            style::append_meta(&body, &slot.slot);
            style::append_meta(&body, &slot.current);
        }
    }
    dialog.set_extra_child(Some(&body));

    let plan_id = plan
        .plan_id
        .clone()
        .filter(|_| outcome.is_some_and(PlanOutcome::is_committable));
    match plan_id {
        Some(_) => dialog.add_responses(&[
            ("cancel", copy::HARNESS_PREVIEW_CANCEL),
            ("commit", copy::HARNESS_PREVIEW_CONFIRM),
        ]),
        None => dialog.add_responses(&[("cancel", copy::HARNESS_PREVIEW_CANCEL)]),
    }
    dialog.set_close_response("cancel");

    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response != "commit" {
            return;
        }
        let Some(plan_id) = plan_id.clone() else {
            return;
        };
        commit_plan(&app, &plan_id);
    });
    dialog.present();
}

/// Make the change that was shown.
///
/// A plan id and nothing else. A plan is single-use and expires after ten
/// minutes, and the daemon re-reads the file before writing and refuses one
/// that moved underneath it. Both refusals are reported and the list is read
/// again; neither is planned afresh, because the change the contributor
/// confirmed was worked out against a file that is no longer the file on
/// disk.
fn commit_plan(app: &Rc<App>, plan_id: &str) {
    app.call(
        "harness_commit",
        serde_json::json!({ "plan_id": plan_id }),
        |app, result| {
            if result.is_err() {
                app.toast(copy::PRIVATE_INFERENCE_WRITE_UNCONFIRMED);
            }
            render_harnesses(app);
        },
    );
}

pub fn wire(app: &Rc<App>) {
    // The switch writes on its own: flipping it IS the contributor acting,
    // and there is nothing else on the screen to fill in first.
    let a = Rc::clone(app);
    app.private_inference
        .switch
        .connect_active_notify(move |sw| {
            if a.private_inference.filling.get() {
                return;
            }
            send(&a, sw.is_active(), None);
        });
}

/// Ask the daemon what it is doing, and draw that.
///
/// Read on every refresh rather than latched in this window: the listener
/// can stop without this process doing anything, and a latched answer would
/// keep saying it was running.
pub fn refresh(app: &Rc<App>) {
    app.call("get_settings", serde_json::json!({}), |app, result| {
        let Ok(Ok(settings)) = result.map(serde_json::from_value::<crate::model::Settings>) else {
            return;
        };
        render(app, &settings);
    });
    // A separate read, because it answers a different question: the tools
    // on this computer and what has arrived from them are facts about other
    // programs' files and about the ledger, not about this app's settings.
    render_harnesses(app);
}

/// The switch, and what actually happened underneath it.
///
/// The switch shows what was ASKED FOR and the row beneath shows what
/// happened, because they differ exactly when it matters: a listener that
/// refused to start leaves the switch on, and a screen with only the switch
/// on it would say the thing is running.
pub fn render(app: &Rc<App>, settings: &crate::model::Settings) {
    let view = &app.private_inference;
    view.confirmed.set(Some(settings.private_inference));
    // The tray's action row points off the confirmed switch, so it is
    // written here, where the daemon's own answer arrives, rather than
    // optimistically at the press. It is the switch's position and nothing
    // that paints reads it -- see `App::tray`.
    app.tray.set_answering(settings.private_inference);
    view.switch.set_sensitive(!view.write_pending.get());
    view.filling.set(true);
    view.switch.set_active(settings.private_inference);
    view.filling.set(false);

    let status = &view.status;
    while let Some(child) = status.first_child() {
        status.remove(&child);
    }
    // A daemon that has never heard of the field sends nothing, and the
    // shared table renders that as unreported, separately from an unfamiliar
    // nonempty state. Neither is evidence that a listener stopped.
    let (label, port) = match settings.private_inference_state.as_ref() {
        Some(state) => (state.state.as_str(), state.port),
        None => ("", None),
    };
    status.append(&tone_row(
        copy::private_inference_state_line(label),
        indicator_tone(copy::private_inference_state_tone(label)),
    ));
    // Assembled on the shared side, and empty when there is no port. An
    // empty sentence is drawn as no row at all rather than as a blank line.
    let serving = copy::private_inference_serving_line(port);
    if !serving.is_empty() {
        style::append_meta(status, serving);
    }
}

/// Stop answering model calls, for the tray's one write.
///
/// `false` and not a flip. The tray may reduce what this computer answers
/// and may not enlarge it -- `tray::TrayRequest` has no word for the other
/// direction -- and a flip here would supply one whenever the menu that was
/// pressed had gone stale.
///
/// It goes through [`send`] rather than around it so the one write path
/// keeps its guards: the confirmed-value check, the in-flight check, and the
/// rule that the screen is drawn from the daemon's echo and never from what
/// was asked for.
pub(crate) fn turn_off(app: &Rc<App>) {
    send(app, false, None);
}

/// Write the switch, and render from the daemon's echo.
///
/// Never optimistic: what comes back carries `private_inference_state`, and
/// that is the only thing that knows whether the listener actually started.
/// Turning it on from here also records that the question has been answered,
/// so the offer does not appear on the next launch for a contributor who
/// found the screen themselves.
///
/// `then` runs only after a confirmed echo, and only for the first connect:
/// the connect it plans has nothing to write until the listener is on, and
/// running it on the request rather than the echo would plan against a
/// destination that may not exist.
fn send(app: &Rc<App>, on: bool, then: Option<AfterWrite>) {
    let view = &app.private_inference;
    // The signal fires after the thumb moved. Put back the confirmed value
    // while waiting, so an error cannot leave an unconfirmed request on
    // screen.
    view.filling.set(true);
    view.switch
        .set_active(view.confirmed.get().unwrap_or(false));
    view.filling.set(false);
    if view.confirmed.get().is_none() || view.write_pending.replace(true) {
        return;
    }
    view.switch.set_sensitive(false);
    app.call(
        "set_settings",
        serde_json::json!({
            "private_inference": on,
            "private_inference_offer_seen": true,
        }),
        move |app, result| {
            app.private_inference.write_pending.set(false);
            app.private_inference.switch.set_sensitive(true);
            let confirmed = result.as_ref().is_ok_and(|value| {
                copy::private_inference_write_confirmed(
                    Some(on),
                    value
                        .get("private_inference_offer_seen")
                        .and_then(serde_json::Value::as_bool),
                    value
                        .get("private_inference")
                        .and_then(serde_json::Value::as_bool),
                )
            });
            let settings = result
                .ok()
                .and_then(|value| serde_json::from_value::<crate::model::Settings>(value).ok());
            if let Some(settings) = settings.filter(|_| confirmed) {
                render(app, &settings);
                render_harnesses(app);
                if let Some(then) = then {
                    then(app);
                }
            } else {
                app.toast(copy::PRIVATE_INFERENCE_WRITE_UNCONFIRMED);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = include_str!("private_inference.rs");

    /// The rule the whole screen hangs on: `Clear` is the only tone that may
    /// be painted as working, and it is the only tone that reaches the
    /// palette's `Clear`. Held, Attention and Refused each keep their own
    /// distinct glyph and colour, so none of them can be read as "on".
    #[test]
    fn only_a_clear_tone_is_painted_as_working() {
        for tone in [
            copy::PrivateInferenceTone::Neutral,
            copy::PrivateInferenceTone::Held,
            copy::PrivateInferenceTone::Clear,
            copy::PrivateInferenceTone::Attention,
            copy::PrivateInferenceTone::Refused,
        ] {
            assert_eq!(
                indicator_tone(tone) == Tone::Clear,
                tone.reads_as_working(),
                "{tone:?} is painted as working exactly when it reads as working"
            );
        }
    }

    /// Every non-working tone must also be distinguishable from every other
    /// one -- a screen that collapsed Refused into Held would be answering a
    /// different question than the daemon was asked.
    #[test]
    fn each_tone_keeps_its_own_glyph() {
        let mut seen: Vec<&str> = Vec::new();
        for tone in [
            copy::PrivateInferenceTone::Neutral,
            copy::PrivateInferenceTone::Held,
            copy::PrivateInferenceTone::Clear,
            copy::PrivateInferenceTone::Attention,
            copy::PrivateInferenceTone::Refused,
        ] {
            let glyph = indicator_tone(tone).glyph();
            assert!(
                !seen.contains(&glyph),
                "{tone:?} reuses the glyph {glyph:?}"
            );
            seen.push(glyph);
        }
    }

    /// The state a daemon that has never heard of the field reports, and any
    /// label this build does not know, must not read as working either.
    #[test]
    fn an_unreported_or_unknown_state_is_not_painted_as_working() {
        for label in ["", "unknown_state", "stopping", "port_in_use", "crashed"] {
            let tone = copy::private_inference_state_tone(label);
            assert_ne!(
                indicator_tone(tone),
                Tone::Clear,
                "{label:?} must not be painted as working"
            );
        }
    }

    /// The indicator reads the tone. Nothing in the render path may reach
    /// the requested boolean to decide how the state row is painted -- the
    /// switch says what was asked for, the row says what is true.
    #[test]
    fn the_indicator_is_not_drawn_from_the_requested_setting() {
        let body = SOURCE
            .split("pub fn render(")
            .nth(1)
            .expect("render is in this file")
            .split("\nfn send(")
            .next()
            .expect("render ends before send");
        assert!(body.contains("copy::private_inference_state_line(label)"));
        assert!(body.contains("copy::private_inference_state_tone(label)"));
        // `settings.private_inference` may reach the switch and nothing
        // else; the tone row is built from the daemon's reported state.
        let after_switch = body
            .split("view.filling.set(false);")
            .nth(1)
            .expect("the switch is filled before the status row is built");
        assert!(
            !after_switch.contains("settings.private_inference,"),
            "the status row must not branch on the requested setting"
        );
    }

    /// The one write anything outside this module can ask for turns model
    /// calls OFF, and there is no way to ask for the other direction.
    ///
    /// `send` is private, so `turn_off` is the whole of this screen's write
    /// surface for the rest of the application -- the tray included. That is
    /// the structural half; this is the other half, that `turn_off` writes a
    /// fixed `false` rather than flipping whatever it finds. A flip would
    /// turn a stale tray menu into an enable nobody asked for, and enabling
    /// is the direction that must happen with the sentence about what it
    /// exposes on screen.
    #[test]
    fn the_only_write_reachable_from_outside_stops_answering() {
        let body = SOURCE
            .split("pub(crate) fn turn_off(app: &Rc<App>) {")
            .nth(1)
            .expect("turn_off is in this file")
            .split("\n}")
            .next()
            .expect("turn_off closes");
        assert!(
            body.contains("send(app, false, None)"),
            "turn_off stopped writing false"
        );
        assert!(
            !body.contains("true"),
            "turn_off can reach the on direction"
        );
        // The module's own code, without the tests that quote it.
        let code = SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        assert_eq!(
            code.matches("pub(crate) fn ").count(),
            1,
            "a second write escaped this module"
        );
        assert!(
            !code.contains("pub fn send("),
            "the write path is reachable without going through turn_off"
        );
    }

    /// Nothing on this screen is authored here. Every string literal in the
    /// constructor is a CSS class; the words come from `copy`.
    #[test]
    fn the_screen_authors_no_sentence_of_its_own() {
        let body = SOURCE
            .split("    pub fn new() -> Self {")
            .nth(1)
            .expect("the constructor is in this file")
            .split("\n        Self {")
            .next()
            .expect("the constructor ends by returning Self");
        // Comments stripped first: a comment quoting a sentence reads as a
        // literal, and one with an odd number of quotes shifts the parity so
        // every real literal lands in a skipped position and this passes
        // having checked nothing. Same reason as `the_harness_rows_author_no_wording`.
        let code_only: String = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let found: Vec<&str> = code_only.split('"').skip(1).step_by(2).collect();
        // A floor, because a scan that examines nothing passes. If the
        // constructor is renamed, or the split markers stop matching, the
        // extraction silently yields an empty body and every assertion below
        // runs zero times -- a guard that certifies nothing while looking
        // green. The constructor sets CSS classes, so it always has literals.
        assert!(
            !found.is_empty(),
            "the scan found no literals at all -- did the constructor move or \
             the split markers stop matching?"
        );
        for literal in found {
            assert!(
                literal.starts_with("tc-"),
                "{literal:?} is a sentence written in this view rather than read from copy"
            );
        }
    }

    /// The write records that the question has been answered, so the
    /// first-run offer is not asked again of somebody who found this screen.
    ///
    /// `set_settings` carries both keys in one call: two writes could land
    /// half-applied, and the half that landed would be the switch.
    #[test]
    fn turning_it_on_here_also_answers_the_offer() {
        let body = SOURCE
            .split("fn send(app: &Rc<App>, on: bool,")
            .nth(1)
            .expect("send is in this file");
        assert!(body.contains("\"private_inference\": on"));
        assert!(body.contains("\"private_inference_offer_seen\": true"));
        assert!(
            body.contains("set_settings"),
            "the switch must go through set_settings"
        );
    }

    /// The tone this shell paints a state in comes from the shared table,
    /// not from a second copy written out here.
    ///
    /// Reads this file's own source, the way the routing twin does. The
    /// words could not drift -- they are `pub use`d -- but the branching
    /// could, in three shells, and nothing in this repository would notice.
    ///
    /// Held here rather than on the settings card it used to guard: the
    /// card no longer reads a tone at all, and this screen is the only
    /// place in this shell that does.
    #[test]
    fn the_tone_is_not_branched_on_in_this_shell() {
        let start = SOURCE
            .find("fn indicator_tone(")
            .expect("the tone mapper exists");
        let end = SOURCE[start..].find("\n}\n").expect("its body ends") + start;
        let body = &SOURCE[start..end];
        for spelled in [
            "running_no_backends",
            "running_elsewhere",
            "port_in_use",
            "start_failed",
            "crashed",
        ] {
            assert!(
                !body.contains(spelled),
                "the state is branched on in this shell: {spelled}"
            );
        }
    }

    /// Every state this daemon can report gets a tone, and only one of them
    /// may be painted as working.
    ///
    /// The named value matters more than the mapping: `running_no_backends`
    /// is a listener that answers a health check and can pass no call on,
    /// and a green light over it is the exact thing that state exists to
    /// prevent.
    #[test]
    fn only_a_listener_with_somewhere_to_send_is_painted_clear() {
        let tone = |label: &str| indicator_tone(copy::private_inference_state_tone(label));
        assert_eq!(tone("running"), Tone::Clear);
        assert_eq!(tone("running_no_backends"), Tone::Attention);
        assert_ne!(tone("running_no_backends"), Tone::Clear);
        assert_eq!(tone("running_elsewhere"), Tone::Held);
        assert_eq!(tone("off"), Tone::Neutral);
        assert_eq!(tone("stopping"), Tone::Held);
        assert_ne!(
            copy::private_inference_state_line(""),
            copy::private_inference_state_line("off")
        );
        assert_ne!(
            copy::private_inference_state_line("stopping"),
            copy::private_inference_state_line("off")
        );
        for failure in ["port_in_use", "start_failed", "crashed"] {
            assert_eq!(tone(failure), Tone::Refused, "{failure}");
        }
        // A state a later daemon grows, and a daemon that reports none at
        // all, both claim nothing rather than falling through to the
        // working light.
        assert_eq!(tone("a_state_from_a_later_daemon"), Tone::Neutral);
        assert_eq!(tone(""), Tone::Neutral);
    }

    /// A refusal names the way out, and the way out is the switch -- which
    /// is on this screen, beside the sentence saying so.
    #[test]
    fn a_refusal_on_this_screen_says_what_to_do() {
        for failure in ["port_in_use", "start_failed", "crashed"] {
            let line = copy::private_inference_state_line(failure);
            assert!(line.contains("off and on again"), "{failure}: {line}");
        }
    }

    /// No sentence about a tool is authored in this shell. The words come
    /// from the shared module -- the same one the other two shells reach
    /// across the C ABI -- and a tool's own name comes from the daemon.
    ///
    /// Scanned rather than trusted: this file's entry in the whole-shell
    /// wording ratchet is zero, and this narrows a failure to the function
    /// that grew a sentence.
    #[test]
    fn the_harness_rows_author_no_wording() {
        for opening in [
            "pub fn render_harnesses(",
            "fn harness_card(",
            "fn present_exposure(",
            "fn present_plan(",
        ] {
            let body = SOURCE
                .split(opening)
                .nth(1)
                .unwrap_or_else(|| panic!("{opening} is in this file"))
                .split("\n}\n")
                .next()
                .unwrap_or_else(|| panic!("{opening} closes"));
            // Comment lines are stripped before splitting on quotes.
            //
            // The scan below takes every odd-indexed quote-delimited segment,
            // which is only the literals while the quote count ahead of each
            // one is even. A comment that quotes a sentence -- to explain why
            // it is NOT rendered, say -- otherwise reads as a literal, and one
            // carrying an odd number of quotes shifts the parity so real
            // literals land in the skipped positions and the scan passes
            // having examined nothing.
            let code_only: String = body
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            let found: Vec<&str> = code_only.split('"').skip(1).step_by(2).collect();
            // Per-function floor, for the same reason: an extraction that
            // yields an empty body passes without examining anything. Each of
            // these functions sets at least one CSS class, so zero literals
            // means the scan lost the function, not that the function is
            // clean.
            assert!(
                !found.is_empty(),
                "{opening} yielded no literals -- the scan has lost it"
            );
            for literal in found {
                assert!(
                    !literal.contains(' ') || literal.starts_with("tc-"),
                    "{literal:?} is a sentence written in {opening} rather than read from copy"
                );
            }
        }
    }

    /// Only a tool a call actually arrived from may be painted as working.
    ///
    /// `activity_shared` is the state this rule exists for: two connected
    /// tools that phrase their calls the same way cannot be told apart, so
    /// "one of these two answered" is not "this one is answering".
    #[test]
    fn only_an_answering_harness_is_painted_as_working() {
        use trace_commons_contributor::harness_state::HarnessState;
        assert_eq!(harness_tone(HarnessState::Answering.label()), Tone::Clear);
        for state in [
            HarnessState::NotConnected,
            HarnessState::ConnectedNoCalls,
            HarnessState::ActivityShared,
            HarnessState::Unknown,
        ] {
            assert_ne!(
                harness_tone(state.label()),
                Tone::Clear,
                "{} must not be painted as working",
                state.label()
            );
        }
        // A state a later daemon grows, and a daemon that reports none at
        // all, claim nothing rather than falling through to the working
        // light.
        for label in ["", "a_state_from_a_later_daemon"] {
            assert_ne!(harness_tone(label), Tone::Clear, "{label:?}");
        }
    }

    /// Three states have a sentence, two have none, and connected is not the
    /// sentence answering is.
    ///
    /// A settings file with the right value in it is not evidence that a
    /// single call was ever answered. A surface that showed those two the
    /// same way would tell a contributor their tool was working while it
    /// sent every call somewhere else.
    ///
    /// `activity_shared` and `unknown` draw no line at all, and in
    /// particular `activity_shared` does not borrow the answering sentence:
    /// that sentence's pronoun names the row's own tool, which is exactly
    /// what the state says cannot be worked out. The values are asserted
    /// against the shared table rather than against literals, so this shell
    /// cannot become the second place they are written.
    #[test]
    fn a_connected_harness_and_an_answering_one_read_differently() {
        use trace_commons_contributor::harness_state::HarnessState;
        use trace_commons_contributor::private_inference_copy::private_inference_copy;
        let shared = private_inference_copy();
        for state in [
            HarnessState::NotConnected,
            HarnessState::ConnectedNoCalls,
            HarnessState::Answering,
        ] {
            assert!(
                !harness_line(state.label()).trim().is_empty(),
                "{} has no sentence",
                state.label()
            );
        }
        assert_eq!(
            harness_line(HarnessState::Answering.label()),
            shared.harness_answering
        );
        assert_ne!(
            harness_line(HarnessState::ConnectedNoCalls.label()),
            harness_line(HarnessState::Answering.label())
        );
        for silent in [
            HarnessState::ActivityShared.label(),
            HarnessState::Unknown.label(),
            "a_state_from_a_later_daemon",
        ] {
            assert_eq!(harness_line(silent), "", "{silent} grew a sentence");
            assert_ne!(harness_line(silent), shared.harness_answering);
            assert_ne!(harness_line(silent), shared.harness_connected_nothing_seen);
        }
    }

    /// Every outcome that writes nothing explains itself, and it is the
    /// shared table that says how.
    ///
    /// Four of these five had no sentence at all before, and their preview
    /// opened with a title, a path, no changes and a way out. The values are
    /// asserted against the shared payload rather than against literals, so
    /// this shell cannot become the second place they are written.
    #[test]
    fn every_plan_outcome_that_changes_nothing_says_why() {
        use trace_commons_contributor::harness_state::PlanOutcome;
        use trace_commons_contributor::private_inference_copy::private_inference_copy;
        let shared = private_inference_copy();
        assert_eq!(
            harness_outcome_line(PlanOutcome::Noop.label()),
            shared.harness_plan_nothing_to_change
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::Unparseable.label()),
            shared.harness_unreadable_config
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::NotInstalled.label()),
            shared.harness_not_installed
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::EntryUnusable.label()),
            shared.harness_plan_entry_unusable
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::NoConfigPath.label()),
            shared.harness_plan_no_config_path
        );
        for silent in [
            PlanOutcome::Changes.label(),
            "an_outcome_from_a_later_daemon",
            "",
        ] {
            assert_eq!(harness_outcome_line(silent), "", "{silent} grew a sentence");
        }
    }

    /// A tool that is not on this computer says so, and does not keep a
    /// sentence about settings it does not have.
    #[test]
    fn a_missing_tool_says_so_rather_than_borrowing_the_not_connected_sentence() {
        use trace_commons_contributor::harness_state::HarnessState;
        use trace_commons_contributor::private_inference_copy::private_inference_copy;
        let shared = private_inference_copy();
        assert_ne!(copy::HARNESS_NOT_INSTALLED, shared.harness_not_connected);
        assert_eq!(copy::HARNESS_NOT_INSTALLED, shared.harness_not_installed);
        assert_ne!(
            copy::HARNESS_NOT_INSTALLED,
            harness_line(HarnessState::NotConnected.label())
        );
    }

    /// `activity_shared` cannot happen yet, and the day it can, this fails.
    ///
    /// It is the one state with no sentence of its own. It means a call DID
    /// arrive and we know it did; what we cannot say is which of two tools
    /// made it. The sentence it borrows -- the shared "state unavailable"
    /// one -- understates that, and would be the mirror image of the
    /// fail-open this whole surface exists to prevent.
    ///
    /// What makes borrowing it acceptable today is that the branch is
    /// unreachable: reaching it needs two CONNECTED tools sharing one
    /// protocol family, and the only tools that exist speak different ones.
    /// The catalog channel that would introduce a third is inert.
    ///
    /// So this reads the one place a family is assigned and requires the
    /// families to stay distinct. Add a same-family tool and this test goes
    /// red, which is the moment somebody has to write the real sentence in
    /// the shared module. It is a tripwire, not a style check.
    #[test]
    fn no_two_connectable_tools_share_a_protocol_family() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../trace-commons-contributor/src/harness_state.rs"
        ))
        .expect("the shared crate's source is beside this one");
        let body = source
            .split("pub fn built_in_family(")
            .nth(1)
            .expect("built_in_family is the one place a family is assigned")
            .split("\n}")
            .next()
            .expect("its body ends");
        // Comments stripped first. This tripwire is what forces a sentence to
        // be written the day two tools share a family, so a commented-out or
        // merely discussed `Some("...")` must not be able to trip it, nor to
        // hide a real one by shifting the quote parity.
        let body: String = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut families: Vec<&str> = Vec::new();
        for arm in body.split("Some(").skip(1) {
            let family = arm
                .split('"')
                .nth(1)
                .expect("every assigned family is a literal");
            assert!(
                !families.contains(&family),
                "two tools now share the {family:?} family, so `activity_shared` is reachable. \
                 It needs a sentence of its own in private_inference_copy before this shell \
                 can render it honestly."
            );
            families.push(family);
        }
        assert!(
            families.len() >= 2,
            "the family table was not read: {families:?}"
        );
    }

    /// An occupied slot is reported as left alone, never as something to
    /// clear or take over.
    ///
    /// The value in that slot is somebody's deliberate choice. The rules
    /// underneath this surface say fill an empty slot and leave a full one
    /// alone, and the only way this shell can break that is by offering a
    /// control that says otherwise. `occupied` reaches the dialog as a
    /// label and never as a response.
    #[test]
    fn an_occupied_slot_is_reported_and_never_offered_as_a_takeover() {
        let body = SOURCE
            .split("fn present_plan(")
            .nth(1)
            .expect("present_plan is in this file")
            .split("\n}\n")
            .next()
            .expect("present_plan closes");
        assert!(
            body.contains("copy::HARNESS_SLOT_TAKEN"),
            "the occupied slots are not reported"
        );
        for spelled in ["overwrite", "force", "take_over", "takeover", "replace"] {
            assert!(!body.contains(spelled), "a takeover path exists: {spelled}");
        }
    }

    /// Nothing is written without the change having been shown first.
    ///
    /// `harness_commit` takes a plan id the daemon minted and nothing else,
    /// so this shell cannot construct a write. What it could still do is
    /// call plan and commit back to back with nothing drawn in between, and
    /// that is what this pins: the single commit call site sits behind the
    /// dialog's confirm response.
    #[test]
    fn a_change_is_never_committed_without_the_preview_being_shown() {
        let code = SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        assert_eq!(
            code.matches("\"harness_commit\"").count(),
            1,
            "harness_commit is called from more than one place"
        );
        let before = code
            .split("\"harness_commit\"")
            .next()
            .expect("something precedes the commit call");
        assert!(
            before.contains("copy::HARNESS_PREVIEW_CONFIRM"),
            "the commit is reachable without the confirm response being drawn"
        );
    }

    /// A plan that no longer applies is an outcome, not a crash and not a
    /// retry.
    ///
    /// Plans are single-use and expire. The daemon refuses a stale one --
    /// an id that is spent or timed out, or a file that moved underneath it
    /// -- and the answer to both is to say so and read the list again.
    /// Planning again on the contributor's behalf would write a change they
    /// were shown against a file that is no longer the file they saw.
    #[test]
    fn a_refused_commit_is_reported_and_the_list_is_read_again() {
        let body = SOURCE
            .split("fn commit_plan(")
            .nth(1)
            .expect("commit_plan is in this file")
            .split("\n}\n")
            .next()
            .expect("commit_plan closes");
        assert!(
            body.contains("copy::PRIVATE_INFERENCE_WRITE_UNCONFIRMED"),
            "a refused commit says nothing"
        );
        assert!(
            body.contains("render_harnesses"),
            "a refused commit leaves a stale list on screen"
        );
        assert!(
            !body.contains("\"harness_plan\""),
            "a refused commit plans again behind the contributor"
        );
        // Every refusal is the same refusal. The daemon takes the plan out
        // of its store before it re-reads the file and before it writes, so
        // an expired id, a spent one, a file that moved and a write that
        // failed all leave nothing to commit again. Branching on which one
        // it was could only produce an offer to retry that cannot work.
        for label in [
            "harness-plan-unknown",
            "harness-config-changed",
            "harness-commit-failed",
        ] {
            assert!(
                !body.contains(label),
                "a commit refusal is branched on: {label}"
            );
        }
    }

    /// The exposure question is asked once, before the first connect.
    ///
    /// The same predicate the first-run offer uses, and the same write path:
    /// a second rule for when to ask is a second chance to nag somebody who
    /// already answered, and a second write path lets the switch and the
    /// marker fall out of step.
    #[test]
    fn a_connect_that_would_reopen_the_listener_asks_what_it_exposes() {
        // Wider than the first-run offer's own predicate, and deliberately
        // so: the offer is asked once, and this is asked every time a
        // connect would turn answering back on. Somebody who used the kill
        // switch is deciding again.
        assert!(connect_needs_exposure(false));
        assert!(!connect_needs_exposure(true));
        let code = SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests");
        let gate = code
            .split("fn present_exposure(")
            .nth(1)
            .expect("present_exposure is in this file");
        assert!(
            gate.contains("copy::PRIVATE_INFERENCE_OFFER_EXPOSURE"),
            "the exposure sentence is not on the gate"
        );
        // And it writes through the switch's own path, so the switch and
        // the answered marker stay on one code path.
        assert!(
            gate.contains("send(&app,"),
            "the gate writes around the switch's own write path"
        );
        // The only place a connect is planned without the gate in front of
        // it is the continuation the gate itself installs.
        let card = code
            .split("fn harness_card(")
            .nth(1)
            .expect("harness_card is in this file")
            .split("\n}\n")
            .next()
            .expect("harness_card closes");
        assert!(
            card.contains("connect_needs_exposure(listener_on(&app))"),
            "a connect can be planned without the exposure gate"
        );
    }

    /// The daemon stamps its times with fractional seconds, and this reads
    /// them.
    ///
    /// `to_rfc3339` emits `2026-09-07T12:00:00.123456789Z`, which a strict
    /// second-resolution parse refuses. Getting this wrong costs the whole
    /// last-call line on every row that has one, so both shapes are pinned
    /// rather than assumed.
    #[test]
    fn a_time_with_fractional_seconds_is_read_as_a_time() {
        let now = chrono::Utc::now();
        for stamp in [
            now.to_rfc3339(),
            now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            now.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        ] {
            let seconds = seconds_since(Some(&stamp))
                .unwrap_or_else(|| panic!("{stamp} was not read as a time"));
            assert!(seconds < 60, "{stamp} read as {seconds} seconds ago");
            assert!(
                !copy::harness_last_call_line(Some(seconds)).is_empty(),
                "{stamp} produced no line"
            );
        }
        // Absent and unreadable both draw no line rather than a wrong one.
        assert_eq!(seconds_since(None), None);
        assert_eq!(seconds_since(Some("")), None);
        assert_eq!(seconds_since(Some("last tuesday")), None);
        assert!(copy::harness_last_call_line(None).is_empty());
    }

    /// UNKNOWN AND NONE-SPENT ARE DIFFERENT FACTS all the way through the
    /// decode, and only one of them draws a line.
    ///
    /// The shapes below are every way the daemon can decline to answer:
    /// `known` false, no block at all -- which is what a daemon older than
    /// the release that reports it sends -- and the contradiction of a
    /// claimed figure with no number. All three must read as not known, and
    /// none of them may draw the sentence a measured zero draws.
    #[test]
    fn an_unmeasured_amount_draws_nothing_and_a_measured_zero_draws_zero() {
        let list = |spend: &str| {
            serde_json::from_value::<crate::model::HarnessList>(
                serde_json::from_str(&format!(
                    r#"{{"catalog_present":false,"harnesses":[]{spend}}}"#
                ))
                .expect("the fixture is JSON"),
            )
            .expect("the list decodes")
        };

        for absent in [
            "",
            r#","spend":{"known":false,"micros":null}"#,
            r#","spend":{"known":true,"micros":null}"#,
        ] {
            let spend = list(absent).spend.micros();
            assert_eq!(spend, None, "{absent} claimed a figure");
            assert!(
                copy::harness_spend_line(spend).is_empty(),
                "{absent} drew a line"
            );
        }

        let zero = list(r#","spend":{"known":true,"micros":0}"#).spend.micros();
        assert_eq!(zero, Some(0));
        let zero_line = copy::harness_spend_line(zero);
        assert!(zero_line.contains("$0.00"), "{zero_line}");
        assert!(!zero_line.is_empty());

        let some = list(r#","spend":{"known":true,"micros":1230000}"#)
            .spend
            .micros();
        assert_eq!(some, Some(1_230_000));
        assert!(copy::harness_spend_line(some).contains("$1.23"));
    }
}
