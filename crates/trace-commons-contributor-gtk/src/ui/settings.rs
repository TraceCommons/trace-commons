//! Settings: pause, projects, permissions, and the local record of what was
//! armed.
//!
//! Everything here has a CLI equivalent, which on Linux is the point: a
//! capability reachable only through this window is a capability a headless
//! contributor does not have. Nothing in this view is the only way to do
//! anything.
//!
//! ## Two visual languages, on purpose
//!
//! Everything above the public-profile section is the native palette --
//! hairlines, the warm ground, the `tc_*` tokens. The public-profile block
//! and the go-public dialog are drawn in the community brand instead: a 2px
//! black frame, Helvetica, uppercase display type, mint. That seam is the
//! design (`DESIGN-SPEC.md` §5.6, §5.7, §7.3): the black frame is the exact
//! boundary of what becomes public, so it is not smoothed into GNOME
//! conventions. It is drawn from [`community_brand`], the one stylesheet
//! History's Community panel shares.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::community_brand;
use super::style::{self, Tone, space};
use crate::copy;
use crate::model::{Project, Settings, Status};

/// Byte budget for the bio, from §5.6 ("280 bytes, plaintext, no HTML").
/// Bytes, not characters: the field is a plaintext byte budget on the wire,
/// so a multi-byte character costs what it actually costs.
const BIO_BYTE_LIMIT: usize = 280;

/// What §5.6 draws, as data.
///
/// Every value in the mockup -- the handle `manian`, the bio, `74/280`,
/// `On the roster since May 12, 2026` -- is a fixture. Nothing here is
/// hardcoded; the panel renders whatever this carries and does not render
/// at all when there is nothing to render.
///
/// Filled from `get_public_profile`, which reports the daemon's local
/// cache of the last claim this device made. There is no
/// `GET /v1/community/profile` to read the server's own row from, so this
/// is what a shell has, and it is a cache: it says what this machine last
/// published, not what the roster holds this second.
#[derive(Debug, Clone)]
pub struct PublicProfile {
    pub handle: String,
    pub bio: String,
    pub on_roster_since: Option<chrono::DateTime<chrono::Utc>>,
    /// Where "View public profile" goes. §7.2 records the target as
    /// unspecified, so no URL is invented here -- the affordance appears
    /// only when something supplies one.
    pub public_url: Option<String>,
}

pub struct SettingsView {
    pub root: gtk::Box,
    connection: gtk::Label,
    /// Holds the one chip that says whether this machine is connected.
    /// §5.4 draws it as a status pill, and §7.3 requires colour AND glyph
    /// AND words, which is what `style::tag` is.
    connection_chip: gtk::Box,
    /// The three check rows of §5.4's Connection section. Filled from
    /// `get_settings`, never from a guess.
    connection_checks: gtk::Box,
    pause_button: gtk::Button,
    projects: gtk::Box,
    /// The three `set_settings` knobs, built once and only ever refilled.
    ///
    /// Everything else on this screen is torn down and rebuilt on each
    /// render, which is fine for labels and fatal for a control: a refresh
    /// runs on every daemon event, and rebuilding a spin button would
    /// destroy the one the contributor is in the middle of typing into.
    quiescence_minutes: gtk::SpinButton,
    approval_hold_seconds: gtk::SpinButton,
    /// Shown only while the hold is zero. See `copy::KNOB_HOLD_ZERO`.
    approval_hold_note: gtk::Label,
    digest_hours: gtk::SpinButton,
    /// Set while `render_knobs` is writing the daemon's own values into
    /// those three, so the `value-changed` they emit is not mistaken for a
    /// contributor turning a dial and echoed straight back as a write.
    filling_knobs: std::cell::Cell<bool>,
    autostart_body: gtk::Label,
    autostart_row: gtk::Box,
    autostart_switch: gtk::Switch,
    /// The background-app-registration row, filled in once
    /// `portal::spawn_request`'s classification lands -- see
    /// `render_background`. `None` until then, which is why it starts on
    /// `copy::PORTAL_STATUS_CHECKING` rather than a guess.
    background_state: RefCell<Option<crate::portal::BackendState>>,
    background_body: gtk::Label,
    /// The public-profile section, rebuilt whole on each render because it
    /// is two different surfaces -- the native opt-in toggle and the brand
    /// panel -- rather than one surface in two states.
    public: gtk::Box,
    /// `None` means "not on the roster". Filled from `get_public_profile`
    /// on every refresh, and from the answer to a claim or a withdrawal
    /// the moment one lands. See `PublicProfile`.
    public_profile: RefCell<Option<PublicProfile>>,
    audit: gtk::Box,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsView {
    pub fn new() -> Self {
        community_brand::install();

        // §4.1's Linux content padding is `16px 20px 22px`, and §5.4 puts
        // the settings sections 18px apart. 18 is not a step in
        // `style::space`; `XL` (20) is the nearest one above it.
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::XL)
            .margin_top(space::L)
            .margin_bottom(space::XXL)
            .margin_start(space::XL)
            .margin_end(space::XL)
            .build();

        // What is running, and the one control that changes it, in one
        // card. These two facts belong together: reading "a background
        // watcher is running" and then hunting for Pause somewhere else is
        // the state and its control being separated for no reason.
        //
        // §5.4 heads this section "Connection" and opens it with a chip
        // and three check rows; the sentences underneath are this shell's
        // and say what the chip cannot -- which process is watching, and
        // what is still true while it is paused.
        content.append(&style::section(copy::CONNECTION_HEADING));
        let state_card = style::card(gtk::Orientation::Vertical, space::M);
        let connection_chip = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        connection_chip.set_halign(gtk::Align::Start);
        state_card.append(&connection_chip);
        let connection = gtk::Label::builder().xalign(0.0).wrap(true).build();
        connection.add_css_class("tc-body");
        state_card.append(&connection);
        let connection_checks = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        state_card.append(&connection_checks);
        let pause_button = gtk::Button::with_label("Pause");
        pause_button.add_css_class("tc-quiet");
        pause_button.set_halign(gtk::Align::Start);
        state_card.append(&pause_button);
        content.append(&state_card);

        content.append(&style::section("Projects"));
        let projects = style::card(gtk::Orientation::Vertical, space::M);
        content.append(&projects);

        content.append(&style::section("How it behaves"));
        let knobs = style::card(gtk::Orientation::Vertical, space::M);
        // Ranges, not free numbers. The daemon accepts any `u64` for all
        // three, and a contributor who typed one into a text field could
        // set a quiet period of a year and then wonder why nothing was
        // ever offered. The bounds below are wide enough to be nobody's
        // ceiling and narrow enough that no value inside them breaks the
        // loop -- and the hold's floor of zero is a real setting (no undo
        // window), which is why it is not one.
        let quiescence_minutes = knob_row(
            &knobs,
            copy::KNOB_QUIESCENCE_TITLE,
            copy::KNOB_QUIESCENCE_UNIT,
            1.0,
            240.0,
        );
        let approval_hold_seconds = knob_row(
            &knobs,
            copy::KNOB_HOLD_TITLE,
            copy::KNOB_HOLD_UNIT,
            0.0,
            300.0,
        );
        // A hold of zero is not a small undo window, it is none, and that
        // is a different statement from the number beside it. It gets its
        // own line, shown only when it is true.
        let approval_hold_note = gtk::Label::builder()
            .label(copy::KNOB_HOLD_ZERO)
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        approval_hold_note.add_css_class("tc-caveat");
        knobs.append(&approval_hold_note);
        let digest_hours = knob_row(
            &knobs,
            copy::KNOB_DIGEST_TITLE,
            copy::KNOB_DIGEST_UNIT,
            1.0,
            24.0,
        );
        let knobs_note = gtk::Label::builder()
            .label(copy::KNOBS_NOTE)
            .xalign(0.0)
            .wrap(true)
            .build();
        knobs_note.add_css_class("tc-caveat");
        knobs.append(&knobs_note);
        content.append(&knobs);

        content.append(&style::section(copy::AUTOSTART_HEADING));
        let autostart_card = style::card(gtk::Orientation::Vertical, space::M);
        let autostart_body = gtk::Label::builder().xalign(0.0).wrap(true).build();
        autostart_body.add_css_class("tc-body");
        autostart_card.append(&autostart_body);
        let autostart_row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let autostart_switch_label = gtk::Label::builder()
            .label(copy::AUTOSTART_XDG_LABEL)
            .xalign(0.0)
            .hexpand(true)
            .build();
        autostart_switch_label.add_css_class("tc-body");
        let autostart_switch = gtk::Switch::builder().halign(gtk::Align::End).build();
        // The switch is reachable by keyboard; the label beside it is not a
        // control, so it is pointed at the switch for a screen reader
        // rather than left as loose text.
        autostart_switch
            .update_property(&[gtk::accessible::Property::Label(copy::AUTOSTART_XDG_LABEL)]);
        autostart_row.append(&autostart_switch_label);
        autostart_row.append(&autostart_switch);
        autostart_card.append(&autostart_row);
        // Same card, not a new section: whether this desktop can list
        // Trace Commons as a background app is the same topic as whether
        // it starts automatically, and the two facts read better together
        // than split across headings.
        let background_body = gtk::Label::builder().xalign(0.0).wrap(true).build();
        background_body.add_css_class("tc-meta");
        background_body.set_text(copy::PORTAL_STATUS_CHECKING);
        autostart_card.append(&background_body);
        content.append(&autostart_card);

        // §5.6. Kept visually separate from everything above it because it
        // is the only thing on this screen that grants no data use at all
        // -- and, once opted in, because it is drawn in a different visual
        // language entirely. `render_public` fills it, section header
        // included: on the roster the brand panel carries its own heading
        // and a native eyebrow above it would say the same words twice.
        let public = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        content.append(&public);

        content.append(&style::section("What has been changed on this machine"));
        let audit = style::card(gtk::Orientation::Vertical, space::XS);
        content.append(&audit);

        // §5.4: "prose column, kept narrow on purpose" -- `max-width:520px`.
        let clamp = adw::Clamp::builder()
            .maximum_size(520)
            .tightening_threshold(440)
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
            connection,
            connection_chip,
            connection_checks,
            pause_button,
            projects,
            quiescence_minutes,
            approval_hold_seconds,
            approval_hold_note,
            digest_hours,
            filling_knobs: std::cell::Cell::new(false),
            autostart_body,
            autostart_row,
            autostart_switch,
            background_state: RefCell::new(None),
            background_body,
            public,
            public_profile: RefCell::new(None),
            audit,
        }
    }
}

pub fn wire(app: &Rc<App>) {
    let a = Rc::clone(app);
    app.settings.pause_button.connect_clicked(move |_| {
        let paused = a
            .status
            .borrow()
            .as_ref()
            .map(|s| s.paused)
            .unwrap_or(false);
        if paused {
            a.call("resume", serde_json::json!({}), |app, _| app.refresh());
        } else {
            offer_pause(&a);
        }
    });

    // The three knobs, each writing exactly the one key it owns.
    //
    // `set_settings` rejects an object holding a key it does not recognise
    // rather than ignoring it, and it applies every key present -- so
    // sending all three on every turn of one dial would make this window
    // re-assert two values it was not asked to change, over whatever a
    // concurrent `trace-commons-contributor settings --set` had just
    // written. One key per write keeps this window's edits to what the
    // contributor actually edited.
    wire_knob(app, &app.settings.quiescence_minutes, "quiescence_secs", 60);
    wire_knob(
        app,
        &app.settings.approval_hold_seconds,
        "approval_hold_secs",
        1,
    );
    wire_knob(
        app,
        &app.settings.digest_hours,
        "digest_interval_secs",
        3600,
    );

    render_autostart(app);
    render_public(app);
    let a = Rc::clone(app);
    app.settings
        .autostart_switch
        .connect_state_set(move |_, wanted| {
            // Only reachable when detection already put the switch in play --
            // see `render_autostart`, which hides this row entirely rather
            // than disabling it when a systemd unit is doing the job. So an
            // event here always means "write or remove the XDG entry",
            // never "fight the systemd unit for control".
            let result = if wanted {
                crate::autostart::enable_xdg_entry()
            } else {
                crate::autostart::disable_xdg_entry()
            };
            if result.is_err() {
                a.toast("That couldn't be changed just now. Nothing else changed either.");
            }
            render_autostart(&a);
            gtk::glib::Propagation::Proceed
        });
}

/// Drain the background-portal probe's answer, once, and render it. The
/// probe is spawned once at app startup (see `App::build`), so this is
/// wired once here too rather than re-run on every settings refresh.
pub fn wire_background_probe(
    app: &Rc<App>,
    rx: async_channel::Receiver<crate::portal::BackendState>,
) {
    render_background(app);
    let a = Rc::clone(app);
    gtk::glib::spawn_future_local(async move {
        if let Ok(state) = rx.recv().await {
            *a.settings.background_state.borrow_mut() = Some(state);
            render_background(&a);
        }
    });
}

/// Pause offers the three durations from the shared spec, backed by
/// `pause {until}` so a timed pause survives this window quitting -- which
/// on Linux it routinely does, since the watcher is usually another
/// process.
fn offer_pause(app: &Rc<App>) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some("Pause contributing?"),
        Some("Nothing is queued or sent while paused. Anything already waiting stays waiting."),
    );
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        ("hour", "For 1 hour"),
        ("tomorrow", "Until tomorrow morning"),
        ("forever", "Until I turn it back on"),
    ]);
    dialog.set_close_response("cancel");
    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        let params = match response {
            "hour" => serde_json::json!({
                "until": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()
            }),
            "tomorrow" => serde_json::json!({ "until": tomorrow_morning().to_rfc3339() }),
            "forever" => serde_json::json!({}),
            _ => return,
        };
        app.call("pause", params, |app, _| app.refresh());
    });
    dialog.present();
}

/// 9am local time on the next day, expressed as an instant. The daemon
/// rejects a timestamp already in the past, so this is always at least a
/// few hours out.
fn tomorrow_morning() -> chrono::DateTime<chrono::Utc> {
    let local = chrono::Local::now();
    let tomorrow = local.date_naive().succ_opt().unwrap_or(local.date_naive());
    tomorrow
        .and_hms_opt(9, 0, 0)
        .map(|naive| naive.and_utc())
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(12))
}

/// Which of the two autostart mechanisms is in force, and say so plainly.
/// Local filesystem state, not the daemon's, so this needs no round trip
/// and can be called eagerly at wire time.
fn render_autostart(app: &Rc<App>) {
    match crate::autostart::detect() {
        crate::autostart::Mechanism::SystemdUnit => {
            app.settings
                .autostart_body
                .set_text(copy::AUTOSTART_SYSTEMD_BODY);
            app.settings.autostart_row.set_visible(false);
        }
        crate::autostart::Mechanism::XdgEntry { enabled } => {
            app.settings
                .autostart_body
                .set_text(copy::AUTOSTART_XDG_BODY);
            app.settings.autostart_row.set_visible(true);
            // Programmatic sync from disk, not a click. GTK's
            // `state-set` signal fires only on user interaction with the
            // switch, not from `set_active`, so this cannot loop back
            // into the handler above and re-write the file it just read.
            app.settings.autostart_switch.set_active(enabled);
        }
    }
}

/// The background-app-registration row. Reads live filesystem detection for
/// the systemd signal (cheap, and the source of truth `render_autostart`
/// already uses) and the stored probe answer for the portal signal, so the
/// two are never rendered from stale copies of each other.
fn render_background(app: &Rc<App>) {
    let systemd_unit_installed = matches!(
        crate::autostart::detect(),
        crate::autostart::Mechanism::SystemdUnit
    );
    let text = match *app.settings.background_state.borrow() {
        Some(state) => copy::portal_status_line(state, systemd_unit_installed),
        None => copy::PORTAL_STATUS_CHECKING,
    };
    app.settings.background_body.set_text(text);
}

pub fn render_status(app: &Rc<App>, status: &Status) {
    let hosting = app.worker.hosts_the_loop();
    let connection = if status.paused {
        "Paused. Nothing is being queued or sent."
    } else if hosting {
        "This window is doing the watching. Closing it stops that."
    } else {
        "A background watcher is running separately. It keeps going when this window closes."
    };
    let connected = if status.logged_in {
        "Connected to Trace Commons."
    } else {
        "Not connected. Sessions are still being queued; nothing can be sent yet, and nothing \
         has been lost."
    };
    app.settings
        .connection
        .set_text(&format!("{connection}\n{connected}"));
    app.settings
        .pause_button
        .set_label(if status.paused { "Resume" } else { "Pause" });

    // §5.4 draws only the connected chip. The other half of the same fact
    // has to be visible too, and §7.3 will not let it be a colour on its
    // own, so both states are a chip with a glyph and words.
    let chip = &app.settings.connection_chip;
    while let Some(child) = chip.first_child() {
        chip.remove(&child);
    }
    chip.append(&if status.logged_in {
        style::tag(copy::CONNECTED, Tone::Clear)
    } else {
        style::tag(copy::NOT_CONNECTED, Tone::Attention)
    });
}

/// §5.4's three check rows. Every one of them is a configured-or-not fact
/// from `get_settings`; not one of them can carry a path or a credential,
/// because the contract keeps both off the wire.
fn render_connection_checks(app: &Rc<App>, settings: &Settings) {
    let view = &app.settings.connection_checks;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }
    view.append(&check_row(
        if settings.claude_root_configured {
            copy::CHECK_CLAUDE_SET
        } else {
            copy::CHECK_CLAUDE_DEFAULT
        },
        settings.claude_root_configured,
        None,
    ));
    view.append(&check_row(
        if settings.codex_root_configured {
            copy::CHECK_CODEX_SET
        } else {
            copy::CHECK_CODEX_DEFAULT
        },
        settings.codex_root_configured,
        None,
    ));
    // The scan's row keeps the sentence this shell already had under it.
    // "Configured" is not the fact a contributor needs -- that message text
    // leaves the machine is, and it is the kind of consequence this
    // product states rather than implies.
    view.append(&check_row(
        if settings.near_ai_configured {
            copy::CHECK_SCAN_SET
        } else {
            copy::CHECK_SCAN_UNSET
        },
        settings.near_ai_configured,
        Some(if settings.near_ai_configured {
            "Message text is scanned by a third party before anything is sent."
        } else {
            "Local scrubbing only."
        }),
    ));
}

/// One check row: a tone glyph, the fact in words, and optionally the
/// consequence underneath. Not a control -- §6.9 calls these "not
/// interactive" -- so it is a label, not a disabled checkbox.
fn check_row(label: &str, satisfied: bool, note: Option<&str>) -> gtk::Box {
    let tone = if satisfied {
        Tone::Clear
    } else {
        Tone::Neutral
    };
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
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
    // Read as one statement rather than as a glyph and a stray sentence.
    row.update_property(&[gtk::accessible::Property::Label(label)]);
    column.append(&row);
    if let Some(note) = note {
        let note_label = gtk::Label::builder()
            .label(note)
            .xalign(0.0)
            .wrap(true)
            .margin_start(space::L)
            .build();
        note_label.add_css_class("tc-meta");
        column.append(&note_label);
    }
    column
}

pub fn refresh(app: &Rc<App>) {
    app.call("list_projects", serde_json::json!({}), |app, result| {
        let projects: Vec<Project> = result
            .ok()
            .and_then(|v| serde_json::from_value(v.get("projects").cloned()?).ok())
            .unwrap_or_default();
        render_projects(app, &projects);
    });
    app.call("get_settings", serde_json::json!({}), |app, result| {
        let Ok(value) = result else { return };
        let Ok(settings) = serde_json::from_value::<Settings>(value) else {
            return;
        };
        render_connection_checks(app, &settings);
        render_knobs(app, &settings);
    });
    // The roster state, from the daemon rather than from what this window
    // last did. A failure -- `not-logged-in` on a device that has never
    // enrolled, most of all -- draws the off-the-roster surface, which is
    // the true one: an unenrolled device has claimed nothing.
    app.call(
        "get_public_profile",
        serde_json::json!({}),
        |app, result| {
            set_public_profile(app, result.ok().and_then(|v| parse_public_profile(&v)));
        },
    );
    app.call(
        "list_audit",
        serde_json::json!({ "limit": 20 }),
        |app, result| {
            let entries = result
                .ok()
                .and_then(|v| v.get("entries").cloned())
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let view = &app.settings.audit;
            while let Some(child) = view.first_child() {
                view.remove(&child);
            }
            if entries.is_empty() {
                let empty = gtk::Label::builder()
                    .label("Nothing has been changed.")
                    .xalign(0.0)
                    .build();
                empty.add_css_class("tc-meta");
                view.append(&empty);
            }
            for entry in entries {
                // `action`, `project_label` and `detail` are fixed labels by
                // contract, so this line can carry no path or token.
                let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let project = entry
                    .get("project_label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let at = entry.get("at").and_then(|v| v.as_str()).unwrap_or("");
                // The instant is a figure, so it is set as one, and the
                // column of them lines up down the card.
                let line = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
                let when = gtk::Label::builder().label(at).xalign(0.0).build();
                when.add_css_class("tc-ledger");
                when.add_css_class("tc-neutral");
                line.append(&when);
                let what = gtk::Label::builder()
                    .label(format!("{}  {project}", audit_sentence(action)))
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                what.add_css_class("tc-meta");
                line.append(&what);
                view.append(&line);
            }
        },
    );
}

fn audit_sentence(action: &str) -> &'static str {
    match action {
        "armed-auto-upload" => "Automatic contributing turned on for",
        "disarmed-auto-upload" => "Automatic contributing turned off for",
        "queue-bulk-approved" => "The whole queue was approved",
        "consent-scopes-changed" => "Permissions changed",
        "near-ai-notice-acknowledged" => "The extra privacy scan was confirmed",
        _ => "Changed",
    }
}

/// Projects, named by `project_id` on the wire and by `project_label` on
/// screen. A path never appears in either direction.
/// The modes a row may be set to: display name beside the wire name.
///
/// `auto_upload` is absent for the unresolvable bucket. `Policy` refuses it
/// there in two independent places, so offering it invited a contributor to
/// select "Contribute automatically" and have the daemon silently decline --
/// believing they had armed something that cannot be armed. Silencing still
/// works, so `ignore` stays.
///
/// Paired rather than positional, and lifted out here so the pairing is
/// testable without a display. The old code carried the mapping twice, as
/// hardcoded indices into a list assumed to be the same length for every
/// row; one shorter row turns that into a control that sets the wrong mode.
fn mode_choices(is_unresolved_bucket: bool) -> Vec<(&'static str, &'static str)> {
    let mut choices: Vec<(&'static str, &'static str)> = vec![("Ask me first", "notify_only")];
    if !is_unresolved_bucket {
        choices.push(("Contribute automatically", "auto_upload"));
    }
    choices.push(("Never offer this one", "ignore"));
    choices
}

fn render_projects(app: &Rc<App>, projects: &[Project]) {
    let view = &app.settings.projects;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }

    // Armed projects are listed first and never collapsed away, so a
    // contributor always knows what is contributing without being asked.
    let armed: Vec<&Project> = projects
        .iter()
        .filter(|p| p.mode == "auto_upload")
        .collect();
    // Armed means "contributes without asking", which is the strongest
    // thing this window can be set to do. It gets the attention tone when
    // anything is armed and the clear tone when nothing is, plus a glyph
    // and words, because it is the state a person most needs to be able to
    // check at a glance.
    let armed_summary = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    armed_summary.append(&if armed.is_empty() {
        style::tag("Nothing is armed", Tone::Clear)
    } else {
        style::tag(&format!("{} armed", armed.len()), Tone::Attention)
    });
    let armed_line = gtk::Label::builder()
        .label(if armed.is_empty() {
            "Every session is offered to you first.".to_string()
        } else {
            armed
                .iter()
                .map(|p| p.project_label.clone())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    armed_line.add_css_class("tc-body");
    armed_summary.append(&armed_line);
    view.append(&armed_summary);

    for project in projects {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        // Name over note, so the row reads as one thing with a property.
        // Every row but the bucket has no note and this collapses to the
        // single label it was.
        let column = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
        column.set_hexpand(true);
        // The wire label for the unresolvable bucket is the slug
        // `unknown-project`, which is an identifier and not a name. Screen 5
        // says the same thing about the same bucket, so it says it in the
        // same words rather than in a second wording of one fact.
        let display_label = if project.is_unresolved_bucket {
            copy::ONBOARD_WATCH_UNKNOWN_LABEL
        } else {
            project.project_label.as_str()
        };
        let label = gtk::Label::builder()
            .label(display_label)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        label.add_css_class("tc-body");
        column.append(&label);
        if project.is_unresolved_bucket {
            let note = gtk::Label::builder()
                .label(copy::ONBOARD_WATCH_UNKNOWN_NOTE)
                .xalign(0.0)
                .wrap(true)
                .build();
            note.add_css_class("tc-meta");
            column.append(&note);
        }
        row.append(&column);

        // The modes this row may actually be set to, display name beside the
        // wire name.
        //
        // `auto_upload` is omitted for the unresolvable bucket. `Policy`
        // refuses it there in two independent places, so offering it invited
        // a contributor to select "Contribute automatically" and have the
        // daemon silently decline -- believing they had armed something that
        // cannot be armed. Silencing still works, so `Ignore` stays.
        //
        // Paired rather than positional. The old code carried the mapping
        // twice, once in `set_selected` and once in `selected_notify`, as
        // hardcoded indices into a list assumed to be the same length for
        // every row. The moment one row is shorter that assumption turns
        // into a control that sets the wrong mode, silently, on the screen
        // that decides what leaves the machine.
        let choices = mode_choices(project.is_unresolved_bucket);

        let display: Vec<&str> = choices.iter().map(|(shown, _)| *shown).collect();
        let modes = gtk::DropDown::from_strings(&display);
        // The project name sits in a separate label, so the control has to
        // say what it controls on its own.
        modes.update_property(&[gtk::accessible::Property::Label(&format!(
            "How to treat {display_label}"
        ))]);
        let selected = choices
            .iter()
            .position(|(_, wire)| *wire == project.mode)
            .unwrap_or(0);
        modes.set_selected(selected as u32);
        row.append(&modes);
        view.append(&row);

        let wire_modes: Vec<String> = choices
            .iter()
            .map(|(_, wire)| (*wire).to_string())
            .collect();
        let app = Rc::clone(app);
        let project = project.clone();
        modes.connect_selected_notify(move |dropdown| {
            let Some(wanted) = wire_modes.get(dropdown.selected() as usize) else {
                return;
            };
            if *wanted == project.mode {
                return;
            }
            if wanted == "auto_upload" {
                confirm_arming(&app, &project, dropdown.clone());
            } else {
                set_mode(&app, &project.project_id, wanted);
            }
        });
    }
}

/// Arming is allowed from the app, but never silently.
fn confirm_arming(app: &Rc<App>, project: &Project, dropdown: gtk::DropDown) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some(&copy::arming_heading(&project.project_label)),
        Some(copy::ARMING_BODY),
    );
    dialog.add_responses(&[("cancel", copy::NOT_NOW), ("arm", copy::ARMING_CONFIRM)]);
    dialog.set_response_appearance("arm", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");

    let app = Rc::clone(app);
    let project_id = project.project_id.clone();
    let previous = match project.mode.as_str() {
        "ignore" => 2u32,
        _ => 0u32,
    };
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response == "arm" {
            set_mode(&app, &project_id, "auto_upload");
        } else {
            dropdown.set_selected(previous);
        }
    });
    dialog.present();
}

fn set_mode(app: &Rc<App>, project_id: &str, mode: &str) {
    app.call(
        "set_project_mode",
        serde_json::json!({ "project_id": project_id, "mode": mode }),
        |app, result| {
            if result.is_err() {
                app.toast("That couldn't be changed just now. Nothing else changed either.");
            }
            app.refresh();
        },
    );
}

/// Send one knob's value to the daemon whenever the contributor changes it.
///
/// `scale` converts the unit on screen into the unit on the wire -- minutes
/// and hours are what a person sets, seconds are what the contract takes.
///
/// The result is not applied optimistically. `set_settings` answers with the
/// settings as they now stand, and that answer is what fills the controls
/// back in, so a refused write leaves the knob showing what the daemon
/// actually holds rather than what this window hoped it would.
fn wire_knob(app: &Rc<App>, spin: &gtk::SpinButton, key: &'static str, scale: u64) {
    let app = Rc::clone(app);
    spin.connect_value_changed(move |spin| {
        // `render_knobs` writing the daemon's own answer back in is not a
        // contributor turning a dial, and echoing it would be this window
        // arguing with the daemon about a value it just supplied.
        if app.settings.filling_knobs.get() {
            return;
        }
        let seconds = knob_seconds(spin.value_as_int(), scale);
        app.call(
            "set_settings",
            serde_json::json!({ key: seconds }),
            |app, result| match result {
                Ok(value) => {
                    if let Ok(settings) = serde_json::from_value::<Settings>(value) {
                        render_knobs(app, &settings);
                    }
                }
                // The label is a fixed one by contract; it is not shown,
                // because none of them is a sentence a contributor can act
                // on. What they need to know is that nothing changed.
                Err(_) => {
                    app.toast(copy::KNOB_NOT_CHANGED);
                    refresh(app);
                }
            },
        );
    });
}

/// The unit on screen, converted to the unit on the wire.
///
/// Clamped at zero rather than cast: `set_settings` takes a `u64`, and a
/// negative value would wrap to an enormous quiet period rather than being
/// refused. The spin buttons cannot produce one today; this is the reason
/// they cannot start to.
fn knob_seconds(shown: i32, scale: u64) -> u64 {
    shown.max(0) as u64 * scale
}

/// The unit on the wire, converted back to the unit on screen.
///
/// Integer division on purpose: the controls step in whole minutes and
/// whole hours, so a value between two steps is shown as the step below it
/// -- and is only ever written back if the contributor turns that dial,
/// which is them choosing the rounded value rather than this window
/// choosing it for them.
fn knob_shown(seconds: u64, scale: u64) -> f64 {
    (seconds / scale.max(1)) as f64
}

/// One editable knob: an eyebrow, a spin button, and the unit it counts in.
///
/// Appended to `card` here rather than returned as a row, because the only
/// thing the caller ever wants back is the control it has to read and
/// write.
fn knob_row(card: &gtk::Box, title: &str, unit: &str, lower: f64, upper: f64) -> gtk::SpinButton {
    let row = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
    row.append(&style::eyebrow(title));
    let control = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let spin = gtk::SpinButton::with_range(lower, upper, 1.0);
    spin.set_numeric(true);
    // Snaps to the step, so a typed half-minute cannot become a value the
    // daemon was never offered.
    spin.set_snap_to_ticks(true);
    spin.set_valign(gtk::Align::Center);
    // The eyebrow above is the visible label, and it is not a control, so
    // it is pointed at the spin button for a screen reader rather than
    // left as loose text over an unnamed field.
    spin.update_property(&[gtk::accessible::Property::Label(title)]);
    control.append(&spin);
    let unit_label = gtk::Label::builder().label(unit).xalign(0.0).build();
    unit_label.add_css_class("tc-body");
    unit_label.set_valign(gtk::Align::Center);
    control.append(&unit_label);
    row.append(&control);
    card.append(&row);
    spin
}

/// Fill the three knobs from the daemon's own answer.
///
/// Never from this shell's idea of what it just wrote: `set_settings`
/// returns the settings as they now stand, and both that and `get_settings`
/// land here, so what is on screen is always what the daemon holds.
///
/// `local_notifications` is deliberately not offered. It is a
/// `set_settings` key, and turning it on would have the daemon render its
/// own OS notifications alongside the ones this window already posts --
/// two notifications for one digest, on the desktop of whoever went
/// looking for the setting. A window that cannot avoid that should not
/// offer the switch.
fn render_knobs(app: &Rc<App>, settings: &Settings) {
    let view = &app.settings;
    // Writing a value emits `value-changed`. Marked as ours so the handler
    // does not echo the daemon's own answer straight back at it.
    view.filling_knobs.set(true);
    view.quiescence_minutes
        .set_value(knob_shown(settings.quiescence_secs, 60));
    view.approval_hold_seconds
        .set_value(knob_shown(settings.approval_hold_secs, 1));
    view.digest_hours
        .set_value(knob_shown(settings.digest_interval_secs, 3600));
    view.filling_knobs.set(false);

    view.approval_hold_note
        .set_visible(settings.approval_hold_secs == 0);

    // The extra privacy scan and the two session folders used to be
    // restated here as well. §5.4 puts all three in the Connection section
    // as check rows, and they are configured-or-not facts about what this
    // machine is wired to rather than knobs about how it behaves, so they
    // moved rather than being duplicated. See `render_connection_checks`,
    // which kept the scan's consequence sentence intact.
}

// --- The public surface, §5.6 and §5.7 ---------------------------------
//
// Two surfaces, not two states of one: off the roster, this is a native
// toggle that grants nothing; on it, it is a panel drawn in the community
// brand. The change of visual language IS the statement -- §7.3 makes the
// black frame the exact boundary of what becomes public -- so the two are
// built separately rather than restyled into each other.

/// Hand the view a public profile, or the absence of one, and redraw.
pub fn set_public_profile(app: &Rc<App>, profile: Option<PublicProfile>) {
    *app.settings.public_profile.borrow_mut() = profile;
    render_public(app);
}

/// Read the daemon's profile shape.
///
/// All three profile methods answer with the same object -- that is
/// deliberate on the daemon's side, so a client parses one thing whichever
/// call it made -- and all three land here.
///
/// `on_roster` is the daemon's own verdict and is what decides, rather than
/// this window inferring one from the presence of a handle: the field
/// exists to answer exactly this question, and a shell that answered it
/// some other way would be a second opinion about who is public.
fn parse_public_profile(value: &serde_json::Value) -> Option<PublicProfile> {
    if !value
        .get("on_roster")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    Some(PublicProfile {
        handle: value.get("handle").and_then(|v| v.as_str())?.to_string(),
        // Absent and empty are the same thing here: no bio was published.
        bio: value
            .get("bio")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        on_roster_since: value
            .get("public_since")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok()),
        // Null by contract today: the daemon knows the origin it uploads
        // to, not the origin the community site serves profiles from, and
        // says so rather than inventing a link that would not resolve. Read
        // anyway, so the affordance appears the day something supplies one.
        public_url: value
            .get("public_url")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// What to say about a claim the server accepted.
///
/// `handle_persisted` is NOT whether the claim worked. The server has
/// taken the handle by the time this flag exists at all; the flag reports
/// whether the daemon managed to write its local copy of it. So both
/// branches report a published profile, and the false branch adds the
/// weaker thing that is actually true -- that this window will show the
/// contributor as unlisted again until the next successful save.
fn published_sentence(handle_persisted: bool) -> &'static str {
    if handle_persisted {
        copy::PROFILE_PUBLISHED
    } else {
        copy::PROFILE_PUBLISHED_NOT_CACHED
    }
}

/// The mirror, for a withdrawal the server accepted.
fn left_roster_sentence(handle_persisted: bool) -> &'static str {
    if handle_persisted {
        copy::PROFILE_LEFT_ROSTER
    } else {
        copy::PROFILE_LEFT_ROSTER_NOT_CACHED
    }
}

/// The bio as the wire wants it.
///
/// An empty box is `null`, not `""`: the `PUT` replaces the whole profile,
/// so "leave the bio alone" is not something the server can be asked for,
/// and the daemon refuses an omitted `bio` outright rather than guessing.
/// An empty box is a contributor saying they want no bio, and that is what
/// this sends.
fn bio_param(text: &str) -> serde_json::Value {
    match text.trim() {
        "" => serde_json::Value::Null,
        bio => serde_json::Value::String(bio.to_string()),
    }
}

/// Claim or update the handle, and report what happened.
///
/// `done` is handed the daemon's fixed error label on a refusal and
/// `None` on success, so the two call sites can put the refusal where the
/// contributor can act on it: the dialog keeps it beside the field being
/// corrected, the panel toasts it. Nothing here validates the handle
/// first -- the daemon and the server share one copy of those rules, and a
/// second copy in this window is how a handle this shell accepts becomes
/// one the server refuses.
fn claim_handle<F>(app: &Rc<App>, handle: &str, bio: &str, done: F)
where
    F: FnOnce(&Rc<App>, Option<String>) + 'static,
{
    app.call(
        "set_public_profile",
        serde_json::json!({ "handle": handle, "bio": bio_param(bio) }),
        move |app, result| match result {
            Ok(value) => {
                // Rendered from the daemon's answer rather than from what
                // this window sent: the handle it stored is the validated
                // display form, which is trimmed, and the roster date is
                // the server's.
                set_public_profile(app, parse_public_profile(&value));
                let persisted = value
                    .get("handle_persisted")
                    .and_then(|v| v.as_bool())
                    // A build that did not report the flag is treated as
                    // having persisted: the alternative is warning about a
                    // local cache miss that may not have happened, on a
                    // profile that is public either way.
                    .unwrap_or(true);
                app.toast(published_sentence(persisted));
                done(app, None);
            }
            Err(label) => done(app, Some(label)),
        },
    );
}

/// Withdraw the handle from the roster.
fn leave_roster(app: &Rc<App>) {
    app.call(
        "clear_public_profile",
        serde_json::json!({}),
        |app, result| match result {
            Ok(value) => {
                set_public_profile(app, parse_public_profile(&value));
                let persisted = value
                    .get("handle_persisted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                app.toast(left_roster_sentence(persisted));
            }
            // Its own sentence, not the claim one: after a failed
            // withdrawal the handle is still published, and "nothing was
            // published" would read as the opposite.
            Err(label) => app.toast(&copy::roster_leave_failure_sentence(&label)),
        },
    );
}

fn render_public(app: &Rc<App>) {
    let view = &app.settings.public;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }
    // Cloned out of the cell before building, so a handler that redraws
    // this section cannot run while the borrow is still live.
    let profile = app.settings.public_profile.borrow().clone();
    match profile {
        Some(profile) => view.append(&public_profile_panel(app, &profile)),
        None => {
            view.append(&style::section(copy::PUBLIC_HEADING));
            view.append(&public_opt_in_row(app));
        }
    }
    let footnote = gtk::Label::builder()
        .label(copy::PUBLIC_FOOTNOTE)
        .xalign(0.0)
        .wrap(true)
        .build();
    // §5.6 sets this footnote in native type outside the panel: it is this
    // window talking about the public surface, not part of it. Linux takes
    // the tertiary ink.
    footnote.add_css_class("tc-meta");
    footnote.add_css_class("tc-tertiary");
    view.append(&footnote);
}

/// Off the roster. The toggle §5.4 names, and nothing else -- turning it on
/// opens the consent dialog rather than doing anything.
fn public_opt_in_row(app: &Rc<App>) -> gtk::Box {
    let card = style::card(gtk::Orientation::Vertical, space::M);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let label = gtk::Label::builder()
        .label(copy::LIST_HANDLE_PUBLICLY)
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .build();
    label.add_css_class("tc-body");
    // Off. §7.3: nothing optional is pre-checked, and this is the most
    // optional thing in the window.
    let toggle = gtk::Switch::builder()
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .active(false)
        .build();
    toggle.update_property(&[gtk::accessible::Property::Label(copy::LIST_HANDLE_PUBLICLY)]);
    row.append(&label);
    row.append(&toggle);
    card.append(&row);

    let app = Rc::clone(app);
    toggle.connect_state_set(move |toggle, wanted| {
        // Only the on direction asks anything: the off direction is
        // unreachable by hand, since the panel replaces this row once a
        // profile exists, and the dialog itself puts the switch back.
        if wanted {
            offer_going_public(&app, toggle.clone());
        }
        gtk::glib::Propagation::Proceed
    });
    card
}

/// On the roster: §5.6's brand panel.
fn public_profile_panel(app: &Rc<App>, profile: &PublicProfile) -> gtk::Box {
    // §5.6's panel gap is 14px; `space::M` (12) is the nearest step.
    let panel = gtk::Box::new(gtk::Orientation::Vertical, space::M);
    panel.add_css_class("tc-brand-panel");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    let title = brand_display(copy::PUBLIC_HEADING);
    title.set_hexpand(true);
    header.append(&title);
    let trailing = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
    trailing.set_halign(gtk::Align::End);
    if let Some(url) = &profile.public_url {
        trailing.append(&brand_link(copy::VIEW_PUBLIC_PROFILE, url));
    }
    if let Some(since) = profile.on_roster_since {
        let since_label = brand_label(&copy::on_roster_since(
            &since
                .with_timezone(&chrono::Local)
                .format("%B %-d, %Y")
                .to_string(),
        ));
        since_label.set_xalign(1.0);
        trailing.append(&since_label);
    }
    header.append(&trailing);
    panel.append(&header);

    let handle_group = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
    handle_group.append(&brand_label(copy::HANDLE_LABEL));
    let handle = gtk::Entry::builder().text(profile.handle.as_str()).build();
    handle.add_css_class("tc-brand-field");
    handle.add_css_class("tc-brand-mono");
    handle.update_property(&[gtk::accessible::Property::Label(copy::HANDLE_LABEL)]);
    handle_group.append(&handle);
    panel.append(&handle_group);

    let bio_group = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
    bio_group.append(&brand_label(copy::BIO_LABEL));
    let bio_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bio_frame.add_css_class("tc-brand-field");
    let bio = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .build();
    bio.add_css_class("tc-brand-bio");
    bio.buffer().set_text(&profile.bio);
    bio.update_property(&[gtk::accessible::Property::Label(copy::BIO_LABEL)]);
    bio_frame.append(&bio);
    bio_group.append(&bio_frame);
    let counter = brand_label(&bio_counter(&profile.bio));
    counter.set_xalign(1.0);
    bio_group.append(&counter);
    // The counter tracks the buffer rather than the value the panel was
    // built from. What happens at and above the limit is drawn nowhere in
    // the spec, so this counts, says so, and refuses nothing.
    let counter_handle = counter.clone();
    bio.buffer().connect_changed(move |buffer| {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        counter_handle.set_label(&bio_counter(text.as_str()));
    });
    panel.append(&bio_group);

    // §5.6's button pair, at a 10px gap; `space::S` (8) is the nearest step.
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let save = brand_button(copy::SAVE_PROFILE, true);
    let leave = brand_button(copy::LEAVE_ROSTER, false);
    buttons.append(&save);
    buttons.append(&leave);
    panel.append(&buttons);

    // Save re-publishes the whole profile, because that is what the `PUT`
    // does: the handle and the bio as they stand in these two fields, both
    // of them, every time. There is no partial update to offer.
    {
        let app = Rc::clone(app);
        let handle_field = handle.clone();
        let bio_buffer = bio.buffer();
        save.connect_clicked(move |_| {
            let text = bio_buffer.text(&bio_buffer.start_iter(), &bio_buffer.end_iter(), false);
            claim_handle(
                &app,
                handle_field.text().as_str(),
                text.as_str(),
                |app, refusal| {
                    if let Some(label) = refusal {
                        app.toast(&copy::profile_failure_sentence(&label));
                    }
                },
            );
        });
    }
    {
        let app = Rc::clone(app);
        leave.connect_clicked(move |_| leave_roster(&app));
    }
    panel
}

/// "74/280", from what is actually in the buffer.
fn bio_counter(bio: &str) -> String {
    format!("{}/{BIO_BYTE_LIMIT}", bio.len())
}

/// §5.7. Going public is a consent dialog, not a toggle flip.
///
/// It is an `adw::Window` rather than an `adw::MessageDialog` because its
/// body is a brand surface: a message dialog would set the prose in the
/// native palette, and the whole point of this screen is that the public
/// surface does not look like the window around it.
fn offer_going_public(app: &Rc<App>, toggle: gtk::Switch) {
    let dialog = adw::Window::builder()
        .transient_for(&app.window)
        .modal(true)
        // §4.6: the Linux dialog is drawn at 560px.
        .default_width(560)
        .resizable(false)
        .build();

    let header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new(copy::GO_PUBLIC_TITLE, ""))
        .build();

    // §5.7: `padding:20px; gap:16px`, on a pure brand ground.
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::L)
        .margin_top(space::XL)
        .margin_bottom(space::XL)
        .margin_start(space::XL)
        .margin_end(space::XL)
        .build();
    body.add_css_class("tc-brand-surface");

    let headline = gtk::Label::builder()
        .label(copy::GO_PUBLIC_HEADLINE.to_uppercase())
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(16)
        .build();
    headline.add_css_class("tc-brand-dialog-title");
    body.append(&headline);

    // The two columns sit inside one frame split by a single hairline:
    // what is published and what never is are the same object seen from
    // both sides, not two separate claims.
    let columns = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .homogeneous(true)
        .build();
    columns.add_css_class("tc-brand-box");
    columns.append(&consent_column(
        copy::PUBLISHED_HEADING,
        copy::PUBLISHED_BODY,
        true,
    ));
    columns.append(&consent_column(
        copy::NEVER_HEADING,
        copy::NEVER_BODY,
        false,
    ));
    body.append(&columns);

    // The handle itself, and the optional bio. They are inside the consent
    // dialog rather than behind it because the thing being consented to is
    // this exact string becoming public: a contributor cannot meaningfully
    // acknowledge "my handle becomes public" and then be asked afterwards
    // what the handle is.
    let handle = gtk::Entry::builder().build();
    handle.add_css_class("tc-brand-field");
    handle.add_css_class("tc-brand-mono");
    handle.update_property(&[gtk::accessible::Property::Label(
        copy::GO_PUBLIC_HANDLE_LABEL,
    )]);
    let handle_group = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
    handle_group.append(&brand_label(copy::GO_PUBLIC_HANDLE_LABEL));
    handle_group.append(&handle);
    body.append(&handle_group);

    let bio = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .build();
    bio.add_css_class("tc-brand-bio");
    bio.update_property(&[gtk::accessible::Property::Label(copy::GO_PUBLIC_BIO_LABEL)]);
    let bio_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bio_frame.add_css_class("tc-brand-field");
    bio_frame.append(&bio);
    let bio_group = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
    bio_group.append(&brand_label(copy::GO_PUBLIC_BIO_LABEL));
    bio_group.append(&bio_frame);
    let counter = brand_label(&bio_counter(""));
    counter.set_xalign(1.0);
    bio_group.append(&counter);
    let counter_handle = counter.clone();
    bio.buffer().connect_changed(move |buffer| {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        counter_handle.set_label(&bio_counter(text.as_str()));
    });
    body.append(&bio_group);

    // A refusal stays in the dialog, next to the field it is about. A toast
    // would land behind a modal window, and the one thing a contributor
    // needs after "that handle is reserved" is the box they typed it into.
    let refusal = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    refusal.add_css_class("tc-brand-body");
    body.append(&refusal);

    // The acknowledgement. Unchecked, and the only thing that unlocks the
    // primary.
    let ack_row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    ack_row.add_css_class("tc-brand-notice");
    let ack = gtk::CheckButton::builder().active(false).build();
    ack.add_css_class("tc-brand-check");
    ack.set_valign(gtk::Align::Start);
    ack.update_property(&[gtk::accessible::Property::Label(
        copy::GO_PUBLIC_ACKNOWLEDGEMENT,
    )]);
    let ack_label = gtk::Label::builder()
        .label(copy::GO_PUBLIC_ACKNOWLEDGEMENT)
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .build();
    ack_label.add_css_class("tc-brand-body");
    ack_row.append(&ack);
    ack_row.append(&ack_label);
    body.append(&ack_row);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    buttons.set_halign(gtk::Align::End);
    let not_now = brand_button(copy::NOT_NOW, false);
    let confirm = brand_button(copy::GO_PUBLIC_CONFIRM, true);
    // Disabled until the acknowledgement is on, which is the rule §5.7
    // states in words at the foot of the same screen.
    confirm.set_sensitive(false);
    buttons.append(&not_now);
    buttons.append(&confirm);
    body.append(&buttons);

    let footnote = gtk::Label::builder()
        .label(copy::GO_PUBLIC_FOOTNOTE)
        .xalign(0.0)
        .wrap(true)
        .build();
    footnote.add_css_class("tc-brand-footnote");
    body.append(&footnote);

    // The acknowledgement gate, plus the one thing the call cannot be made
    // without. Both are the same rule stated twice: the primary does
    // nothing until there is something to consent to and a consent to it.
    let unlock = {
        let confirm = confirm.clone();
        let ack = ack.clone();
        let handle = handle.clone();
        move || confirm.set_sensitive(ack.is_active() && !handle.text().trim().is_empty())
    };
    let on_ack = unlock.clone();
    ack.connect_toggled(move |_| on_ack());
    let on_typed = unlock.clone();
    handle.connect_changed(move |_| on_typed());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-brand-surface");
    content.append(&header);
    content.append(&body);
    dialog.set_content(Some(&content));

    // Closing without a claim leaves the switch off. The switch says
    // whether a handle is on the roster, and abandoning this dialog has
    // put none there -- a switch left on would be this window claiming a
    // listing that does not exist. A successful claim never reaches this:
    // the panel replaces the row the switch lives in.
    dialog.connect_close_request(move |_| {
        toggle.set_active(false);
        gtk::glib::Propagation::Proceed
    });
    let window = dialog.clone();
    not_now.connect_clicked(move |_| window.close());
    let window = dialog.clone();
    let app = Rc::clone(app);
    let bio_buffer = bio.buffer();
    confirm.connect_clicked(move |confirm| {
        let text = bio_buffer.text(&bio_buffer.start_iter(), &bio_buffer.end_iter(), false);
        // Held shut for the round trip, so a second click cannot send a
        // second claim while the first is still in flight.
        confirm.set_sensitive(false);
        refusal.set_visible(false);
        let window = window.clone();
        let confirm = confirm.clone();
        let refusal = refusal.clone();
        claim_handle(
            &app,
            handle.text().as_str(),
            text.as_str(),
            move |_app, label| match label {
                // Claimed. The toast is already up and the panel has
                // already replaced the toggle; this dialog has nothing
                // left to say.
                None => window.close(),
                // Refused, so the dialog stays open on the handle that was
                // refused: this is the only surface where it can be
                // corrected without typing it again.
                Some(label) => {
                    refusal.set_label(&copy::profile_failure_sentence(&label));
                    refusal.set_visible(true);
                    confirm.set_sensitive(true);
                }
            },
        );
    });
    dialog.present();
}

/// One half of §5.7's consent box. The leading column carries the 1px
/// divider; the trailing one does not.
fn consent_column(heading: &str, body: &str, divided: bool) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, space::S);
    column.add_css_class("tc-brand-cell");
    if divided {
        column.add_css_class("tc-brand-divided");
    }
    column.append(&brand_label(heading));
    column.append(&brand_body(body));
    column
}

/// `display.panel`: uppercased here, since GTK 4 CSS has no
/// `text-transform`.
fn brand_display(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-brand-display");
    label
}

/// `label.mono`: the brand's micro-label, on `brand.muted`.
fn brand_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-brand-label");
    label
}

/// `body.brand`: prose inside a brand panel.
fn brand_body(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-brand-body");
    label
}

/// §6.1's brand pair. `primary` is the mint fill; both carry the same
/// frame and the same label type.
fn brand_button(text: &str, primary: bool) -> gtk::Button {
    let button = gtk::Button::with_label(&text.to_uppercase());
    button.add_css_class("tc-brand-button");
    if primary {
        button.add_css_class("tc-brand-primary");
    }
    button
}

/// The brand text link. Underlined through Pango markup rather than CSS,
/// and it hands the address to the desktop's browser -- the one place this
/// window points outside itself. A launch that fails changes nothing, so
/// there is nothing to report and nothing to undo.
fn brand_link(text: &str, url: &str) -> gtk::Button {
    let label = gtk::Label::new(None);
    label.set_markup(&format!(
        "<u>{}</u>",
        gtk::glib::markup_escape_text(&text.to_uppercase())
    ));
    let button = gtk::Button::builder().child(&label).build();
    button.add_css_class("tc-brand-link");
    button.set_halign(gtk::Align::End);
    button.update_property(&[gtk::accessible::Property::Label(text)]);
    let url = url.to_string();
    button.connect_clicked(move |_| {
        let _ =
            gtk::gio::AppInfo::launch_default_for_uri(&url, None::<&gtk::gio::AppLaunchContext>);
    });
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_knob_round_trips_through_the_unit_it_is_shown_in() {
        // The daemon's defaults, which are the values a contributor sees on
        // a machine nobody has changed: 30 minutes quiet, a 10-second hold,
        // 4 hours between interruptions.
        for (seconds, scale, shown) in [(1800u64, 60u64, 30.0), (10, 1, 10.0), (14_400, 3600, 4.0)]
        {
            assert_eq!(knob_shown(seconds, scale), shown);
            assert_eq!(knob_seconds(shown as i32, scale), seconds);
        }
    }

    #[test]
    fn no_shown_value_can_become_an_enormous_one_on_the_wire() {
        // `set_settings` takes a `u64`. A negative value cast rather than
        // clamped would wrap into a quiet period measured in centuries,
        // and the daemon would accept it.
        assert_eq!(knob_seconds(-1, 60), 0);
        assert_eq!(knob_seconds(i32::MIN, 3600), 0);
    }

    #[test]
    fn a_published_profile_is_read_back_off_the_daemons_own_verdict() {
        let profile = parse_public_profile(&serde_json::json!({
            "on_roster": true,
            "handle": "manian",
            "bio": "Ships billing systems by day.",
            "public_since": "2026-05-12T09:00:00Z",
            "public_url": serde_json::Value::Null,
        }))
        .expect("a claimed handle renders the panel");
        assert_eq!(profile.handle, "manian");
        assert_eq!(profile.bio, "Ships billing systems by day.");
        assert!(profile.on_roster_since.is_some());
        // Null by contract: the daemon does not know the origin the
        // community site serves profiles from, so no link is offered.
        assert!(profile.public_url.is_none());
    }

    #[test]
    fn an_unclaimed_handle_draws_the_opt_in_and_not_an_empty_panel() {
        assert!(
            parse_public_profile(&serde_json::json!({
                "on_roster": false,
                "handle": serde_json::Value::Null,
                "bio": serde_json::Value::Null,
                "public_since": serde_json::Value::Null,
                "public_url": serde_json::Value::Null,
            }))
            .is_none()
        );
        // `not-logged-in` arrives as an error rather than as a shape, and
        // an answer this build cannot read must not become a half-drawn
        // panel either.
        assert!(parse_public_profile(&serde_json::json!({})).is_none());
    }

    #[test]
    fn a_cache_write_that_failed_is_never_reported_as_a_failed_claim() {
        // The correctness point of the whole surface. `handle_persisted:
        // false` means the server took the handle and this device did not
        // manage to write its own copy of it -- the profile IS public. A
        // shell that reported that as a failure would tell a contributor
        // their handle is private when it is not.
        let uncached = published_sentence(false);
        assert!(uncached.starts_with("You're on the roster"));
        assert_ne!(uncached, published_sentence(true));
        // And the withdrawal mirror: the row is gone from the server
        // whether or not the local clear stuck.
        assert!(left_roster_sentence(false).starts_with("You've left the roster"));
    }

    #[test]
    fn an_empty_bio_box_is_sent_as_no_bio_rather_than_as_an_empty_one() {
        // The PUT replaces the whole profile and the daemon refuses an
        // omitted `bio` outright, so the empty box has to mean something
        // explicit. It means "no bio".
        assert!(bio_param("").is_null());
        assert!(bio_param("   \n ").is_null());
        assert_eq!(
            bio_param("  Ships billing systems.  "),
            "Ships billing systems."
        );
    }

    #[test]
    fn a_value_between_two_steps_is_shown_as_the_step_below_it() {
        // Never rounded up: showing 2 minutes for a 90-second setting would
        // overstate how long the watcher actually waits.
        assert_eq!(knob_shown(90, 60), 1.0);
        assert_eq!(knob_shown(0, 60), 0.0);
    }

    #[test]
    fn the_unresolvable_bucket_is_never_offered_auto_upload() {
        // The daemon refuses auto_upload for this bucket in two independent
        // places. A control that offers it is inviting a contributor to
        // believe they armed something that cannot be armed.
        let choices = mode_choices(true);
        assert!(
            !choices.iter().any(|(_, wire)| *wire == "auto_upload"),
            "auto_upload must not be offered for the unresolvable bucket"
        );
        // Silencing is still available: the bucket cannot be armed, but it
        // can be told to stop offering.
        assert!(choices.iter().any(|(_, wire)| *wire == "ignore"));
        assert!(choices.iter().any(|(_, wire)| *wire == "notify_only"));
    }

    #[test]
    fn an_ordinary_project_keeps_every_mode() {
        let choices = mode_choices(false);
        let wires: Vec<&str> = choices.iter().map(|(_, wire)| *wire).collect();
        assert_eq!(wires, vec!["notify_only", "auto_upload", "ignore"]);
    }

    #[test]
    fn a_position_yields_the_mode_that_position_shows() {
        // The defect this guards is silent: with a shorter list on one row, a
        // positional mapping sets a mode the contributor did not pick. Index 1
        // is auto_upload on an ordinary row and ignore on the bucket, and each
        // row must resolve against its own list.
        assert_eq!(mode_choices(true)[1].1, "ignore");
        assert_eq!(mode_choices(false)[1].1, "auto_upload");
    }
}
