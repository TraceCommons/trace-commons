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
use crate::copy::SourceTool;
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
    /// The two daily-cap knobs. Same "built once, refilled" rule as the
    /// three above, and the same `filling_knobs` guard covers them -- see
    /// its doc.
    max_uploads_per_day: gtk::SpinButton,
    max_bytes_per_day_mb: gtk::SpinButton,
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
    /// One row per tool, rebuilt on each render: a name and one word.
    routing_tools: gtk::Box,
    /// The declaration. Built once and only ever refilled, for the same
    /// reason the knobs are -- a refresh runs on every daemon event, and
    /// rebuilding these would take the port field out from under whoever
    /// is typing into it.
    routing_switch: gtk::Switch,
    /// What the machine already knows, before anything it is asked.
    ///
    /// One sentence, from the shared source, for both states: a pointer was
    /// published, or none was. A machine without IronWire is the ordinary
    /// machine and gets a sentence rather than an error.
    routing_discovery: gtk::Label,
    /// The one action offered when a pointer was found: turn it on and
    /// check, in one press. Hidden where there is nothing to connect to,
    /// and where something is already declared.
    routing_connect: gtk::Button,
    /// Ask again, for somebody who started IronWire after opening this
    /// window. Offered rather than polled.
    routing_look_again: gtk::Button,
    /// The port and folder, behind a disclosure. Expanded where discovery
    /// found nothing, because there they are the only way to answer.
    routing_override: gtk::Expander,
    routing_port: gtk::SpinButton,
    routing_token_dir: gtk::Entry,
    routing_apply: gtk::Button,
    /// The answer to the last check, shown only once one has been run.
    /// Never filled from a guess: it says what the daemon reported.
    routing_probe: gtk::Label,
    /// The daemon's own three-state view, rebuilt on each status event.
    routing_status: gtk::Box,
    /// What the contributor said about each tool, held so the tool rows can
    /// be repainted from an event that does not carry `Settings`.
    ///
    /// Caching is not tidiness. The word beside each tool is built from two
    /// facts that arrive on **different events** -- the declaration on
    /// `get_settings`, the evidence on the answer to `probe_routed_tools` --
    /// so a render that read one without holding the other would blank the
    /// word on alternate ticks. Both are cached; either event repaints from
    /// both. See `render_tool_rows`.
    routing_modes: RefCell<Option<RoutingModes>>,
    /// What IronWire last said about each tool, and whether it answered at
    /// all. `None` means nothing has been asked yet this run.
    routing_evidence: RefCell<Option<RoutingEvidence>>,
    /// Set while a tool-list call is in flight, so a refresh -- which runs
    /// on every daemon event -- cannot start a second one.
    routing_evidence_pending: std::cell::Cell<bool>,
    /// Set while `render_routing` is writing the daemon's own declaration
    /// into the controls, so the signals that fires are not mistaken for a
    /// contributor declaring something and echoed straight back.
    filling_routing: std::cell::Cell<bool>,
    /// The port a running IronWire published, or `None` for a machine that
    /// published nothing.
    ///
    /// Held rather than written straight into the field because the field
    /// is repainted on every daemon event, and the rule about it is a
    /// precedence rule: a declared port always wins, this fills in only
    /// where there is none, and the conventional number is what is left.
    /// See `render_routing`.
    routing_discovered_port: std::cell::Cell<Option<u16>>,
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

        // The Tools card. It is deliberately one concept: whether what a
        // tool sends is kept private on this machine. The port and the
        // folder underneath are the override for an unusual install, not
        // the front door -- the conventional port is already in the field
        // and the folder box is empty, so the common case is one switch
        // and nothing to fill in.
        content.append(&style::section(copy::TOOLS_HEADING));
        let routing_card = style::card(gtk::Orientation::Vertical, space::M);
        let routing_tools = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        routing_card.append(&routing_tools);
        let routing_intro = gtk::Label::builder()
            .label(copy::IRONWIRE_INTRO)
            .xalign(0.0)
            .wrap(true)
            .build();
        routing_intro.add_css_class("tc-body");
        routing_card.append(&routing_intro);
        let routing_row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let routing_switch_label = gtk::Label::builder()
            .label(copy::IRONWIRE_TOGGLE)
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .build();
        routing_switch_label.add_css_class("tc-body");
        let routing_switch = gtk::Switch::builder().halign(gtk::Align::End).build();
        routing_switch.set_valign(gtk::Align::Center);
        routing_switch.update_property(&[gtk::accessible::Property::Label(copy::IRONWIRE_TOGGLE)]);
        routing_row.append(&routing_switch_label);
        routing_row.append(&routing_switch);
        routing_card.append(&routing_row);
        let routing_status = gtk::Box::new(gtk::Orientation::Vertical, 2);
        routing_card.append(&routing_status);

        // What the machine already knows, before the two fields that ask.
        // IronWire writes a pointer when its daemon binds, so on a machine
        // running it there is nothing here to look up; on a machine
        // without it this sentence says so without saying anything is
        // wrong, because nothing is.
        let routing_discovery = gtk::Label::builder().xalign(0.0).wrap(true).build();
        routing_discovery.add_css_class("tc-body");
        routing_card.append(&routing_discovery);
        let routing_actions = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let routing_connect = gtk::Button::with_label(copy::IRONWIRE_CONNECT);
        routing_connect.set_visible(false);
        routing_actions.append(&routing_connect);
        let routing_look_again = gtk::Button::with_label(copy::IRONWIRE_LOOK_AGAIN);
        routing_look_again.add_css_class("tc-quiet");
        routing_actions.append(&routing_look_again);
        routing_actions.set_halign(gtk::Align::Start);
        routing_card.append(&routing_actions);

        // The port and folder behind a disclosure. They are the override,
        // not the front door -- but only once discovery has supplied the
        // port. Where it has not they are the only way to answer, and
        // `render_routing` opens this.
        let routing_override = gtk::Expander::new(Some(copy::IRONWIRE_OVERRIDE_TITLE));
        let routing_override_box = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        routing_override.set_child(Some(&routing_override_box));
        routing_card.append(&routing_override);
        // 1 rather than 0: port 0 is the ask-the-kernel sentinel, and the
        // daemon refuses it outright, so it is not a number this control
        // may produce.
        let routing_port = knob_row(
            &routing_override_box,
            copy::IRONWIRE_PORT_TITLE,
            "",
            1.0,
            f64::from(u16::MAX),
        );
        routing_port.set_value(f64::from(DEFAULT_IRONWIRE_PORT));
        let routing_port_note = gtk::Label::builder()
            .label(copy::IRONWIRE_PORT_NOTE)
            .xalign(0.0)
            .wrap(true)
            .build();
        routing_port_note.add_css_class("tc-caveat");
        routing_override_box.append(&routing_port_note);
        routing_override_box.append(&style::eyebrow(copy::IRONWIRE_FOLDER_TITLE));
        // Still a text box here, deliberately. The macOS folder control is
        // a chooser because on that platform a directory is readable when
        // the person pointed at it through the system panel, not when the
        // app was told a string. No such rule holds on this one.
        let routing_token_dir = gtk::Entry::new();
        routing_token_dir.update_property(&[gtk::accessible::Property::Label(
            copy::IRONWIRE_FOLDER_TITLE,
        )]);
        routing_override_box.append(&routing_token_dir);
        // Assembled rather than fixed: it names the folder this machine
        // would read when the field is left empty, which is the folder every
        // failure sentence on this card sends a contributor here to name.
        let routing_folder_note = gtk::Label::builder()
            .label(copy::ironwire_folder_note_here())
            .xalign(0.0)
            .wrap(true)
            .build();
        routing_folder_note.add_css_class("tc-caveat");
        routing_override_box.append(&routing_folder_note);
        let routing_apply = gtk::Button::with_label(copy::IRONWIRE_APPLY);
        routing_apply.add_css_class("tc-quiet");
        routing_apply.set_halign(gtk::Align::Start);
        routing_card.append(&routing_apply);
        let routing_probe = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        routing_probe.add_css_class("tc-meta");
        routing_card.append(&routing_probe);
        // Nothing here waits on the app being started again: the daemon
        // swaps the reader when the declaration lands.
        let routing_applies = gtk::Label::builder()
            .label(copy::IRONWIRE_APPLIES_AT_ONCE)
            .xalign(0.0)
            .wrap(true)
            .build();
        routing_applies.add_css_class("tc-caveat");
        routing_card.append(&routing_applies);
        content.append(&routing_card);

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

        // The daily upload budget: a separate card from the timing knobs
        // above (see copy.rs's module doc for why). Ranges are `1` (a
        // contributor throttling their own uploads is legitimate, so no
        // higher floor than "not zero") to the same fixed ceiling
        // `apply_settings_object` enforces server-side -- 1,000 uploads,
        // 5,120 MB -- so a value this control can even produce is a value
        // the daemon will actually accept.
        content.append(&style::section(copy::BUDGET_HEADING));
        let budget = style::card(gtk::Orientation::Vertical, space::M);
        let max_uploads_per_day = knob_row(
            &budget,
            copy::KNOB_MAX_UPLOADS_TITLE,
            copy::KNOB_MAX_UPLOADS_UNIT,
            1.0,
            1_000.0,
        );
        let max_bytes_per_day_mb = knob_row(
            &budget,
            copy::KNOB_MAX_BYTES_TITLE,
            copy::KNOB_MAX_BYTES_UNIT,
            1.0,
            5_120.0,
        );
        let budget_note = gtk::Label::builder()
            .label(copy::BUDGET_NOTE)
            .xalign(0.0)
            .wrap(true)
            .build();
        budget_note.add_css_class("tc-caveat");
        budget.append(&budget_note);
        content.append(&budget);

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
            max_uploads_per_day,
            max_bytes_per_day_mb,
            filling_knobs: std::cell::Cell::new(false),
            autostart_body,
            autostart_row,
            autostart_switch,
            background_state: RefCell::new(None),
            background_body,
            public,
            public_profile: RefCell::new(None),
            audit,
            routing_tools,
            routing_switch,
            routing_discovery,
            routing_connect,
            routing_look_again,
            routing_override,
            routing_port,
            routing_token_dir,
            routing_apply,
            routing_probe,
            routing_status,
            routing_modes: RefCell::new(None),
            routing_evidence: RefCell::new(None),
            routing_evidence_pending: std::cell::Cell::new(false),
            filling_routing: std::cell::Cell::new(false),
            routing_discovered_port: std::cell::Cell::new(None),
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
    // The two budget knobs. `max_uploads_per_day` needs no scale (the
    // control and the wire value are the same unit); `max_bytes_per_day`
    // is shown in MB and scaled up to bytes, the same MB-to-bytes
    // convention the rest of this shell already uses in `model::human_bytes`.
    wire_knob(
        app,
        &app.settings.max_uploads_per_day,
        "max_uploads_per_day",
        1,
    );
    wire_knob(
        app,
        &app.settings.max_bytes_per_day_mb,
        "max_bytes_per_day",
        1_048_576,
    );

    // The declaration. The switch writes on its own -- turning it on IS
    // the contributor acting, and the conventional port is already in the
    // field -- and the button re-writes it after an edit to either field.
    let a = Rc::clone(app);
    app.settings
        .routing_switch
        .connect_active_notify(move |sw| {
            if a.settings.filling_routing.get() {
                return;
            }
            let on = sw.is_active();
            set_routing_sensitivity(&a, on);
            send_routing(&a, on);
        });
    let a = Rc::clone(app);
    app.settings.routing_apply.connect_clicked(move |_| {
        if !a.settings.routing_switch.is_active() {
            return;
        }
        send_routing(&a, true);
    });
    // The shortcut past the two fields: turn it on and check, in one
    // press. It writes the port that is ON SCREEN -- which is the
    // discovered one, or whatever the contributor typed over it -- so a
    // press cannot declare a number different from the one displayed.
    let a = Rc::clone(app);
    app.settings.routing_connect.connect_clicked(move |_| {
        a.settings.filling_routing.set(true);
        a.settings.routing_switch.set_active(true);
        a.settings.filling_routing.set(false);
        set_routing_sensitivity(&a, true);
        send_routing(&a, true);
    });
    let a = Rc::clone(app);
    app.settings
        .routing_look_again
        .connect_clicked(move |_| discover_routing(&a));

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
        concat!("Connected to ", copy::app_name!(), ".")
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

    render_routing_status(app, status);
}

/// The two session-source rows: the sentence, and whether the tone is the
/// satisfied one.
///
/// Branches on `*_source_mode`, never on `*_root_configured`. That boolean
/// is `mode == "watch"` and so is false for `off` as well as for `unset`,
/// and this shell used to render one sentence on that false branch: a
/// contributor who declared Claude Code OFF was told its sessions were being
/// read from the usual place. Nothing is read from an `off` source.
///
/// Split out of the render so it can be asserted on without a GTK widget
/// tree. The words themselves are `trace_commons_contributor::source_copy`'s,
/// because the macOS and Windows shells print this same row.
fn source_check_rows(settings: &Settings) -> Vec<(String, bool)> {
    vec![
        (
            copy::source_check_line(SourceTool::Claude, &settings.claude_source_mode),
            settings.claude_source_mode == "watch",
        ),
        (
            copy::source_check_line(SourceTool::Codex, &settings.codex_source_mode),
            settings.codex_source_mode == "watch",
        ),
    ]
}

/// §5.4's three check rows. Every one of them is a configured-or-not fact
/// from `get_settings`; not one of them can carry a path or a credential,
/// because the contract keeps both off the wire.
fn render_connection_checks(app: &Rc<App>, settings: &Settings) {
    let view = &app.settings.connection_checks;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }
    for (label, satisfied) in source_check_rows(settings) {
        view.append(&check_row(&label, satisfied, None));
    }
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
    tone_row(
        label,
        if satisfied {
            Tone::Clear
        } else {
            Tone::Neutral
        },
        note,
    )
}

/// The same row, for a state that is neither satisfied nor unsatisfied.
/// "Declared, nothing seen yet" is the case a boolean cannot carry: it is
/// not good standing and it is not a fault.
fn tone_row(label: &str, tone: Tone, note: Option<&str>) -> gtk::Box {
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

// --- Tools: the local proxy declaration --------------------------------

/// IronWire's conventional port, shown in the field so nobody has to know
/// it. **Shown is not declared**: nothing is written until the contributor
/// turns the switch on, because `None` means off with no fallback and a
/// displayed default that wrote itself would have this window announce a
/// local service nobody mentioned.
const DEFAULT_IRONWIRE_PORT: u16 = 8463;

/// The `set_settings` key. That call refuses an object holding a key it
/// does not recognise, so a drift here is a silent no-write rather than an
/// error -- which is why a test checks this against the daemon's own
/// serialization rather than against a second copy of the literal.
const ROUTING_SETTINGS_KEY: &str = "ironwire";

/// What `probe_routing` answered, in the three shapes it can answer in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProbeOutcome {
    /// The proxy answered and the credential was accepted.
    Reachable,
    /// The file could not be read, or was read and refused. Carries the
    /// path the daemon reported -- **absent, not null**, when nothing
    /// resolved at all, so this is an `Option` and never an unwrap.
    TokenUnusable(Option<String>),
    /// Nothing usable answered. Carries the port that was tried.
    Unreachable(Option<u16>),
    /// An answer this build cannot read. Claims nothing about the proxy in
    /// either direction.
    Unknown,
}

/// The declaration as `set_settings` takes it, or `null` for off.
///
/// `token_dir` is left out when the box is empty rather than sent as an
/// empty string: the daemon refuses an empty string outright, and absence
/// is what falls back to the conventional location.
fn routing_param(on: bool, port: u16, token_dir: &str) -> serde_json::Value {
    if !on {
        return serde_json::Value::Null;
    }
    let mut declaration = serde_json::Map::new();
    declaration.insert("mode".to_string(), serde_json::json!("watch"));
    declaration.insert("port".to_string(), serde_json::json!(port));
    let dir = token_dir.trim();
    if !dir.is_empty() {
        declaration.insert("token_dir".to_string(), serde_json::json!(dir));
    }
    serde_json::Value::Object(declaration)
}

/// The one-key object `set_settings` is called with.
fn routing_settings_params(on: bool, port: u16, token_dir: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();
    params.insert(
        ROUTING_SETTINGS_KEY.to_string(),
        routing_param(on, port, token_dir),
    );
    serde_json::Value::Object(params)
}

/// What `probe_routing` is asked. Same rule about the empty box.
fn probe_params(port: u16, token_dir: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();
    params.insert("port".to_string(), serde_json::json!(port));
    let dir = token_dir.trim();
    if !dir.is_empty() {
        params.insert("token_dir".to_string(), serde_json::json!(dir));
    }
    serde_json::Value::Object(params)
}

/// Read the daemon's answer, using the daemon's own constants.
fn parse_probe(value: &serde_json::Value) -> ProbeOutcome {
    use trace_commons_contributor::daemon::ipc::{
        PROBE_REACHABLE, PROBE_TOKEN_UNREADABLE, PROBE_UNREACHABLE,
    };
    match value.get("outcome").and_then(serde_json::Value::as_str) {
        Some(PROBE_REACHABLE) => ProbeOutcome::Reachable,
        Some(PROBE_TOKEN_UNREADABLE) => ProbeOutcome::TokenUnusable(
            value
                .get("token_path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        ),
        Some(PROBE_UNREACHABLE) => ProbeOutcome::Unreachable(
            value
                .get("port")
                .and_then(serde_json::Value::as_u64)
                .and_then(|p| u16::try_from(p).ok()),
        ),
        _ => ProbeOutcome::Unknown,
    }
}

/// One outcome, one sentence.
fn probe_line(outcome: &ProbeOutcome) -> String {
    match outcome {
        ProbeOutcome::Reachable => copy::IRONWIRE_PROBE_REACHABLE.to_string(),
        ProbeOutcome::TokenUnusable(path) => copy::ironwire_token_line(path.as_deref()),
        ProbeOutcome::Unreachable(port) => copy::ironwire_unreachable_line(*port),
        ProbeOutcome::Unknown => copy::IRONWIRE_CHECK_UNAVAILABLE.to_string(),
    }
}

/// The tone of the daemon's three states, onto this shell's palette.
///
/// NOT A BRANCH TABLE HERE. Which tone each state reads in is decided once,
/// in `routing_copy`, beside the sentence it goes with -- this only carries
/// that answer onto `style::Tone`, which has two values this surface can
/// never reach. `awaiting_rows` is `Held` and not `Attention`: a reader
/// built a moment ago starts cold by construction, so this is the state a
/// contributor sees immediately after touching anything on this card, and
/// painting it as a fault would accuse a working proxy at exactly that
/// moment.
fn routing_tone(state: &str) -> Tone {
    match copy::ironwire_state_tone(state) {
        copy::StateTone::Held => Tone::Held,
        copy::StateTone::Clear => Tone::Clear,
        copy::StateTone::Neutral => Tone::Neutral,
    }
}

/// Fill the declaration controls from the daemon's own answer, and say one
/// word about each tool.
/// What the contributor said about each of the four tools this card names.
///
/// Held rather than re-read because the tool rows are repainted from the
/// tool-list answer, which arrives on its own event and carries no
/// `Settings`.
/// The declaration switch is deliberately **not** a field. It was the only
/// input to the word before this change, which is what let a contributor
/// read "Private" on the same card as "Nothing answered on port 8463".
/// Turning it off clears the evidence instead, so the words fall back to
/// "not known" rather than being computed from the switch.
#[derive(Clone, Default)]
struct RoutingModes {
    claude: String,
    codex: String,
    gemini: String,
    cline: String,
}

/// What IronWire last answered when asked which tools are pointed at it.
///
/// `outcome` is the same three states the probe reports, and it is what
/// makes a dead proxy stop producing verdicts: on anything but
/// [`ProbeOutcome::Reachable`] every tool reads
/// [`copy::ToolWiring::Unknown`], whatever the switch says.
struct RoutingEvidence {
    outcome: ProbeOutcome,
    /// When this answer was taken, for [`EVIDENCE_BACKSTOP_TTL`]. The
    /// primary invalidation is the probe, not this stamp.
    taken_at: std::time::Instant,
    /// One entry per tool IronWire listed, keyed by its own stable id.
    /// A tool absent from the list -- Gemini CLI and Cline on every machine
    /// today -- is not in this map and gets no verdict.
    tools: std::collections::HashMap<String, ToolRow>,
}

/// One row of IronWire's tool list, reduced to what a word may be built on.
struct ToolRow {
    installed: bool,
    wired: bool,
}

/// IronWire's own stable ids for the four tools this card names.
///
/// `ironwire connect <id>` takes these, and the settings response is keyed
/// by them. Gemini CLI and Cline have no row upstream at all today --
/// neither built-in nor in the catalogue -- which is why they are listed
/// here and expected to be missing rather than left out and quietly
/// defaulted.
const IRONWIRE_TOOL_CLAUDE: &str = "claude";
const IRONWIRE_TOOL_CODEX: &str = "codex";
const IRONWIRE_TOOL_GEMINI: &str = "gemini";
const IRONWIRE_TOOL_CLINE: &str = "cline";

/// What may be said about one tool, from what IronWire answered about it.
///
/// The rules, and why each is where it is:
///
/// * **Nothing answered.** `unreachable` and `token_unreadable` are stable
///   states -- a port nothing is listening on, a credential that is not
///   there or is refused -- so a word built on them would keep asserting
///   while the card underneath says "Nothing answered on port 8463". They
///   yield `Unknown`. This is the original defect, in the one string a
///   person reads.
/// * **The daemon's `awaiting_rows` is deliberately not consulted here.** A
///   proxy installed this morning legitimately reports it, and it flips
///   back to `awaiting_rows` whenever a declaration changes, so letting it
///   downgrade the word would flicker it against a working install.
/// * **Listed but not present.** IronWire saying a tool is not installed,
///   while this app is watching that tool's sessions, is two detectors
///   disagreeing about one machine. That is not evidence for a verdict.
fn tool_wiring(evidence: Option<&RoutingEvidence>, id: &str) -> copy::ToolWiring {
    let Some(evidence) = evidence else {
        return copy::ToolWiring::Unknown;
    };
    if evidence.outcome != ProbeOutcome::Reachable {
        return copy::ToolWiring::Unknown;
    }
    match evidence.tools.get(id) {
        Some(row) if row.wired => copy::ToolWiring::Wired,
        Some(row) if row.installed => copy::ToolWiring::NotWired,
        _ => copy::ToolWiring::Unknown,
    }
}

/// One row per tool: a name, and one word built from both caches.
///
/// The single painter for these rows. Both events that can change a word
/// call it, and it reads the declaration and the evidence together, so
/// neither can arrive and blank what the other established.
fn render_tool_rows(app: &Rc<App>) {
    let view = &app.settings.routing_tools;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }
    let modes = app.settings.routing_modes.borrow();
    let Some(modes) = modes.as_ref() else { return };
    let evidence = app.settings.routing_evidence.borrow();
    for (name, mode, id) in [
        (copy::TOOL_CLAUDE, &modes.claude, IRONWIRE_TOOL_CLAUDE),
        (copy::TOOL_CODEX, &modes.codex, IRONWIRE_TOOL_CODEX),
        (copy::TOOL_GEMINI, &modes.gemini, IRONWIRE_TOOL_GEMINI),
        (copy::TOOL_CLINE, &modes.cline, IRONWIRE_TOOL_CLINE),
    ] {
        let wiring = tool_wiring(evidence.as_ref(), id);
        let word = copy::tool_word(mode, wiring);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let label = gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .build();
        label.add_css_class("tc-body");
        row.append(&label);
        // From the wiring, never from the word. A styling decision that
        // compared the rendered string against the private word would be a
        // text match against a privacy claim, and `Private` is a substring
        // of the denial that must never come back -- the same shape that
        // once let `contains("reachable")` match `"unreachable"` here.
        let tone = match copy::tool_tone(mode, wiring) {
            copy::ToolTone::Clear => Tone::Clear,
            copy::ToolTone::Neutral => Tone::Neutral,
        };
        row.append(&style::tag(word, tone));
        // Read as one statement, not as a name and a stray word.
        row.update_property(&[gtk::accessible::Property::Label(&format!("{name}: {word}"))]);
        view.append(&row);
    }
}

/// Which port the field shows, of the three that can claim it.
///
/// **The contributor's declared port always wins.** A declared port is a
/// human instruction; the pointer is a file on disk that survives the
/// daemon that wrote it, and IronWire removes it only on a clean stop. The
/// failure of letting a stale pointer win is not one refused connection --
/// it is a contributor who declared 8463, whose leftover pointer says
/// 9000, and whose field now shows a number they never typed while the
/// settings file still reads 8463. `ironwire_ledger_for` refuses exactly
/// that substitution on the reading side; this is the same rule on the
/// showing side.
///
/// Discovery fills only where nothing is declared, and the conventional
/// number is the last resort rather than the first. Every one of the three
/// is a *display*: `routing_param` still writes nothing while the switch
/// is off.
///
/// Pure, and separate from the widget it fills, so the precedence can be
/// stated as a table rather than inferred from a running window.
#[must_use]
fn shown_port(declared: Option<u16>, discovered: Option<u16>) -> u16 {
    declared.or(discovered).unwrap_or(DEFAULT_IRONWIRE_PORT)
}

/// The daemon's `discover_routing` method name, spelled once.
const DISCOVER_METHOD: &str = "discover_routing";

/// What a running IronWire published about itself.
///
/// One boolean's worth of distinction, because there is one: a pointer was
/// read, or it was not. Never installed, not running, a version that
/// publishes nothing, a pointer the daemon will not act on -- all of them
/// are the same fact to the contributor and the same next step.
///
/// Carries no token. `discover_routing` returns a path, never a
/// credential, and this shell never opens it either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Discovered {
    /// The loopback port IronWire published, or `None` for nothing found.
    port: Option<u16>,
}

/// Read a `discover_routing` result.
///
/// `found` without a usable port is nothing found: the port is the fact
/// the call exists to supply, and offering to connect to one this shell
/// invented would be worse than asking.
fn parse_discovery(value: &serde_json::Value) -> Discovered {
    if value.get("found").and_then(serde_json::Value::as_bool) != Some(true) {
        return Discovered::default();
    }
    let port = value
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p > 0);
    Discovered { port }
}

/// Ask what the machine already knows, and say so.
///
/// **Writes nothing and reads nothing of the contributor's.** It reads one
/// file IronWire left, learns a port from it, and puts that port in a
/// field. Declaring is still the switch and the two buttons; a discovery
/// that declared on its own would be this window announcing a local
/// service nobody mentioned, which is what the declaration exists to stop.
///
/// A call that did not run degrades to nothing found, which is also what a
/// machine without IronWire answers -- both mean there is nothing to offer,
/// and neither is a fault to render as one.
fn discover_routing(app: &Rc<App>) {
    app.call(DISCOVER_METHOD, serde_json::json!({}), |app, result| {
        let discovered = result.as_ref().map(parse_discovery).unwrap_or_default();
        app.settings.routing_discovered_port.set(discovered.port);
        render_discovery(app);
    });
}

/// The offer: one sentence, and the actions beside it.
///
/// The connect button is offered only where there is something to connect
/// to AND nothing is declared. Where something is declared the switch is
/// already on and the button would be a second Apply.
fn render_discovery(app: &Rc<App>) {
    let port = app.settings.routing_discovered_port.get();
    app.settings
        .routing_discovery
        .set_text(&copy::ironwire_discovery_line(port));
    let declared = app.settings.routing_switch.is_active();
    app.settings
        .routing_connect
        .set_visible(port.is_some() && !declared);
    // Collapsed only once the machine supplied the port. Where it did not,
    // the two fields are the only way to answer, so they stay open: this
    // inverts the default, it does not remove the manual path. Never
    // closed under somebody who opened it -- only ever opened.
    if port.is_none() {
        app.settings.routing_override.set_expanded(true);
    }
}

fn render_routing(app: &Rc<App>, settings: &Settings) {
    let declared = settings
        .ironwire
        .as_ref()
        .is_some_and(|d| d.mode == "watch");

    app.settings.routing_modes.replace(Some(RoutingModes {
        claude: settings.claude_source_mode.clone(),
        codex: settings.codex_source_mode.clone(),
        gemini: settings.gemini_source_mode.clone(),
        cline: settings.cline_source_mode.clone(),
    }));
    if !declared {
        // Nothing is declared, so nothing held about IronWire is still
        // about this machine's current state. Dropped rather than kept,
        // so turning the switch back on cannot paint a stale verdict
        // before the new answer lands.
        app.settings.routing_evidence.replace(None);
    }
    render_tool_rows(app);
    if declared {
        ask_routed_tools(app, settings);
    }

    app.settings.filling_routing.set(true);
    app.settings.routing_switch.set_active(declared);
    // Neither field is refilled while the contributor is in it. Unlike the
    // knobs above, these two are not written on every change -- they are
    // written when Apply is pressed -- so a refresh (which runs on every
    // daemon event) landing mid-edit would otherwise replace a half-typed
    // port with the declared one.
    //
    // The precedence is the rule the whole feature turns on: a declared
    // port always wins, a discovered one fills in only where there is
    // none, and the conventional number is what is left. A pointer is a
    // file that survives the daemon that wrote it -- IronWire removes it
    // only on a clean stop -- so a stale one naming 9000 must not replace
    // a declared 8463. `ironwire_ledger_for` refuses the same substitution
    // on the reading side; this is the same rule on the showing side.
    let shown = shown_port(
        settings.ironwire.as_ref().and_then(|d| d.port),
        app.settings.routing_discovered_port.get(),
    );
    if !app.settings.routing_port.has_focus() {
        app.settings.routing_port.set_value(f64::from(shown));
    }
    let token_dir = settings
        .ironwire
        .as_ref()
        .and_then(|d| d.token_dir.clone())
        .unwrap_or_default();
    if !app.settings.routing_token_dir.has_focus()
        && app.settings.routing_token_dir.text() != token_dir
    {
        app.settings.routing_token_dir.set_text(&token_dir);
    }
    app.settings.filling_routing.set(false);
    set_routing_sensitivity(app, declared);
    render_discovery(app);
}

/// The port and folder fields are the override, and they are live only
/// while the switch is on.
fn set_routing_sensitivity(app: &Rc<App>, on: bool) {
    app.settings.routing_port.set_sensitive(on);
    app.settings.routing_token_dir.set_sensitive(on);
    app.settings.routing_apply.set_sensitive(on);
}

/// The daemon's three-state view of what it is seeing, plus when it last
/// got an answer.
fn render_routing_status(app: &Rc<App>, status: &Status) {
    let view = &app.settings.routing_status;
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }
    let state = status.routing.state.as_str();
    view.append(&tone_row(
        copy::ironwire_state_line(state),
        routing_tone(state),
        // The stamp lives in the running daemon and means "last answered",
        // so it is only shown where it says something: never on a state
        // that has had no answer at all.
        copy::ironwire_last_checked(status.routing.last_refresh_at)
            .as_deref()
            .filter(|_| copy::ironwire_shows_last_checked(state)),
    ));
}

/// Write the declaration, then -- when it is on -- ask the daemon what it
/// found and say so.
///
/// The probe runs only from here: a human pressing a switch or a button.
/// Nothing on the submission path calls it.
fn send_routing(app: &Rc<App>, on: bool) {
    let port = routing_port_value(app);
    let token_dir = app.settings.routing_token_dir.text().to_string();
    // The declaration is about to change, so what IronWire said about the
    // old one is no longer about this machine. Dropped before the write,
    // not after the answer: the words must stop asserting immediately, not
    // once a replacement arrives.
    app.settings.routing_evidence.replace(None);
    if on {
        app.settings.routing_probe.set_text(copy::IRONWIRE_CHECKING);
        app.settings.routing_probe.set_visible(true);
    } else {
        app.settings.routing_probe.set_visible(false);
    }
    app.call(
        "set_settings",
        routing_settings_params(on, port, &token_dir),
        move |app, result| {
            match result {
                Ok(_) if on => check_routing(app, port, token_dir),
                Ok(_) => app.settings.routing_probe.set_visible(false),
                // The error label is a fixed one by contract and is not a
                // sentence anybody can act on. What matters is that
                // nothing changed.
                Err(_) => {
                    app.settings.routing_probe.set_visible(false);
                    app.toast(copy::KNOB_NOT_CHANGED);
                }
            }
            refresh(app);
        },
    );
}

/// Ask IronWire which tools on this machine are pointed at it, and repaint
/// the words from the answer.
///
/// Guarded twice, because `render_routing` runs on every daemon event and
/// this opens a connection: once by the cache (an answer already held is
/// not re-asked) and once by `routing_evidence_pending` (a call already in
/// flight does not start a second). Both are cleared where a contributor
/// changes something, which is the only place a fresh answer is owed.
fn ask_routed_tools(app: &Rc<App>, settings: &Settings) {
    if app.settings.routing_evidence_pending.get() {
        return;
    }
    let fresh = app
        .settings
        .routing_evidence
        .borrow()
        .as_ref()
        .is_some_and(|held| held.taken_at.elapsed() < EVIDENCE_BACKSTOP_TTL);
    if fresh {
        return;
    }
    // The declared values, not the widgets': a refresh can land while
    // somebody is typing into the port box, and the question has to be
    // about the declaration the daemon is actually holding.
    let Some(declaration) = settings.ironwire.as_ref() else {
        return;
    };
    let port = declaration.port.unwrap_or(DEFAULT_IRONWIRE_PORT);
    let token_dir = declaration.token_dir.clone().unwrap_or_default();
    app.settings.routing_evidence_pending.set(true);
    app.call(
        "probe_routed_tools",
        probe_params(port, &token_dir),
        |app, result| {
            app.settings.routing_evidence_pending.set(false);
            // A call that did not run is not a fact about any tool. The
            // cache is left empty so the next render asks again, and every
            // word stays "not known" meanwhile.
            if let Ok(value) = result {
                app.settings
                    .routing_evidence
                    .replace(Some(parse_routed_tools(&value)));
            }
            render_tool_rows(app);
        },
    );
}

/// Read the daemon's tool-list answer.
///
/// Anything unreadable degrades to no evidence rather than to a default:
/// an outcome this build does not know is [`ProbeOutcome::Unknown`], a
/// missing `wired` is not a claim that a tool is wired, and a row without
/// an id is not a row.
fn parse_routed_tools(value: &serde_json::Value) -> RoutingEvidence {
    let mut tools = std::collections::HashMap::new();
    if let Some(rows) = value.get("tools").and_then(serde_json::Value::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            tools.insert(
                id.to_string(),
                ToolRow {
                    installed: row
                        .get("installed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    wired: row
                        .get("wired")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                },
            );
        }
    }
    RoutingEvidence {
        outcome: parse_probe(value),
        taken_at: std::time::Instant::now(),
        tools,
    }
}

/// A backstop, and **not** a freshness policy.
///
/// The two invalidation signals that matter are causal and are not this
/// one: a probe that does not reach the proxy drops the evidence outright
/// (see `check_routing`), and the evidence is dropped again before any
/// declaration is written (see `send_routing`). Between them, every way a
/// contributor can make the held answer wrong already clears it.
///
/// This timer exists only for the case where **neither fires and no probe
/// result arrives at all** -- a machine that slept, a settings card left
/// open overnight, a daemon that stopped ticking. Without it the last good
/// answer stands forever, which is the same defect this change removes,
/// reached by waiting instead of by declaring.
///
/// # Why five minutes, and why it is a multiple rather than a round number
///
/// The interval is only defensible relative to how often this card is
/// re-rendered, because a call can only be made from a render. Every daemon
/// event runs `App::refresh`, and the daemon's own poll loop
/// (`daemon::mod::supervise`) ticks at `poll_interval_secs`, which defaults
/// to 60. So the render cadence is one a minute at rest, and faster when
/// anything is happening.
///
/// A 60-second bound against a 60-second poll is the degenerate case: it
/// expires at the same rate the card re-renders, so it would re-ask on
/// essentially every tick, forever, on a card nobody is touching. Five
/// minutes is five ticks at the default -- long enough that this is a
/// backstop rather than a poll, short enough that a card left open notices
/// a proxy that went away. `the_backstop_is_a_multiple_of_the_poll_it_backs`
/// pins the relationship against the daemon's own default, so a change to
/// either side fails rather than drifts.
///
/// Expiry cannot flicker the word. `ask_routed_tools` does **not** clear the
/// cache when it re-asks -- the stale answer stays on screen until a new one
/// lands -- so an expiry is invisible unless the answer actually changed.
const EVIDENCE_BACKSTOP_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Ask the daemon whether the proxy is there, and print what it answered.
fn check_routing(app: &Rc<App>, port: u16, token_dir: String) {
    app.call(
        "probe_routing",
        probe_params(port, &token_dir),
        |app, result| {
            let outcome = match &result {
                Ok(value) => parse_probe(value),
                Err(_) => ProbeOutcome::Unknown,
            };
            let line = match result {
                Ok(_) => probe_line(&outcome),
                // The check itself did not run. Not a fact about the
                // proxy, so it does not send anybody to look at a port.
                Err(_) => copy::IRONWIRE_CHECK_UNAVAILABLE.to_string(),
            };
            // The card must not carry a verdict above a sentence that
            // contradicts it. This is the defect in its original form: a
            // contributor whose proxy was dead read a confident word on the
            // same card as "Nothing answered on port 8463". The probe is
            // what establishes reachability here, so anything but a
            // reachable answer drops the evidence the words are built from
            // and every tool falls back to "not known".
            if outcome != ProbeOutcome::Reachable {
                app.settings.routing_evidence.replace(None);
                render_tool_rows(app);
            }
            app.settings.routing_probe.set_text(&line);
            app.settings.routing_probe.set_visible(true);
        },
    );
}

/// The port field, as a port. Clamped rather than cast: the control cannot
/// produce a value outside the range today, and this is why it cannot
/// start to.
fn routing_port_value(app: &Rc<App>) -> u16 {
    app.settings
        .routing_port
        .value_as_int()
        .clamp(1, i32::from(u16::MAX)) as u16
}

pub fn refresh(app: &Rc<App>) {
    // Asked before anything is offered, and asked once rather than on
    // every daemon event: this reads a file, and a settings screen that
    // repolled it on every queue change would be going looking on a timer.
    // Somebody who starts IronWire after this window opened presses the
    // button beside the sentence, which re-asks unconditionally.
    if app.settings.routing_discovered_port.get().is_none() {
        discover_routing(app);
    }
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
        render_routing(app, &settings);
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
    view.max_uploads_per_day
        .set_value(knob_shown(settings.max_uploads_per_day, 1));
    view.max_bytes_per_day_mb
        .set_value(knob_shown(settings.max_bytes_per_day, 1_048_576));
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

    fn settings_with_modes(claude: &str, codex: &str) -> Settings {
        serde_json::from_value(serde_json::json!({
            "claude_source_mode": claude,
            "codex_source_mode": codex,
            // Sent by the daemon and deliberately unread by this shell. Set
            // to the value it carries for a watched source, so a row that
            // went back to reading it would still look right for `watch`
            // and wrong for the other two -- which is the bug, not a
            // coincidence the test should hide.
            "claude_root_configured": claude == "watch",
            "codex_root_configured": codex == "watch",
        }))
        .expect("the settings blob parses")
    }

    /// The three modes are three different rows, and the `off` row does not
    /// say what the `unset` row says.
    ///
    /// `off` and `unset` shared a sentence -- "sessions read from the usual
    /// place" -- because the row branched on `*_root_configured`, which is
    /// `mode == "watch"`. Nothing is read from an `off` source, so that was
    /// a false statement on the one screen a contributor checks to confirm
    /// it. `unset` is NOT the same fact: an undeclared claude or codex is
    /// scanned at its conventional location, so its row must keep saying so.
    #[test]
    fn each_source_mode_gets_its_own_row() {
        let watch = source_check_rows(&settings_with_modes("watch", "watch"));
        let unset = source_check_rows(&settings_with_modes("unset", "unset"));
        let off = source_check_rows(&settings_with_modes("off", "off"));

        assert_eq!(watch[0].0, "Claude Code sessions folder set");
        assert_eq!(unset[0].0, "Claude Code sessions read from the usual place");
        assert_eq!(
            off[0].0,
            "Claude Code marked not used, so nothing is opened for it"
        );
        assert_eq!(
            off[1].0,
            "Codex marked not used, so nothing is opened for it"
        );

        for (a, b) in [(&watch, &unset), (&watch, &off), (&unset, &off)] {
            assert_ne!(a[0].0, b[0].0, "two modes render the same row");
            assert!(
                !b[0].0.contains(a[0].0.as_str()) && !a[0].0.contains(b[0].0.as_str()),
                "one row's sentence contains another's: {:?} / {:?}",
                a[0].0,
                b[0].0
            );
        }
        // Only `watch` is the satisfied tone. `off` is a choice rather than
        // a fault, but it is not a folder this app was pointed at.
        assert_eq!(
            (watch[0].1, unset[0].1, off[0].1),
            (true, false, false),
            "the tone stopped tracking the mode"
        );
    }

    /// The two sources are read independently. One tool declared off must
    /// not change what the other tool's row says.
    #[test]
    fn one_source_being_off_does_not_speak_for_the_other() {
        let rows = source_check_rows(&settings_with_modes("off", "unset"));
        assert_eq!(
            rows[0].0,
            "Claude Code marked not used, so nothing is opened for it"
        );
        assert_eq!(rows[1].0, "Codex sessions read from the usual place");
        assert_eq!((rows[0].1, rows[1].1), (false, false));
    }

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

    /// Nothing is written until the contributor acts.
    ///
    /// The port field shows IronWire's conventional number so nobody has
    /// to know it, and a shown default that became a declaration would
    /// have this window announce a local service the contributor never
    /// mentioned. Off is `null`, which the daemon reads as off with no
    /// fallback.
    #[test]
    fn a_shown_default_is_not_a_declaration() {
        assert_eq!(
            routing_param(false, DEFAULT_IRONWIRE_PORT, ""),
            serde_json::Value::Null
        );
        assert_eq!(
            routing_param(false, DEFAULT_IRONWIRE_PORT, "/home/x/.ironwire"),
            serde_json::Value::Null
        );
    }

    /// Ground truth from outside this window: the value it sends is parsed
    /// back by the daemon's own type, so a shape this shell invented could
    /// not pass.
    #[test]
    fn the_declaration_round_trips_into_the_daemons_own_type() {
        use trace_commons_contributor::daemon::settings::IronWireDeclaration;
        let declared: IronWireDeclaration =
            serde_json::from_value(routing_param(true, 8463, "/home/x/iw")).expect("parses");
        assert_eq!(
            declared,
            IronWireDeclaration::Watch {
                port: 8463,
                token_dir: Some(std::path::PathBuf::from("/home/x/iw")),
            }
        );
    }

    /// An empty box is "I did not say", not "the empty directory". The
    /// daemon refuses an empty string outright, and absence is what falls
    /// back to the conventional location.
    #[test]
    fn an_empty_folder_box_is_sent_as_no_folder() {
        for shown in ["", "   ", "\t"] {
            let param = routing_param(true, 8463, shown);
            assert!(
                param.get("token_dir").is_none(),
                "{shown:?} became a declared folder: {param}"
            );
        }
        // And a folder that was typed is sent trimmed, not verbatim.
        assert_eq!(
            routing_param(true, 8463, "  /home/x/iw  ")["token_dir"],
            serde_json::json!("/home/x/iw")
        );
    }

    /// The key `set_settings` takes, checked against the daemon's own
    /// serialization rather than against a literal repeated here.
    /// `set_settings` refuses unknown keys, so a drift is a silent
    /// no-write.
    #[test]
    fn the_settings_key_is_the_one_the_daemon_serializes() {
        use trace_commons_contributor::daemon::settings::{DaemonSettings, IronWireDeclaration};
        let settings = DaemonSettings {
            ironwire: Some(IronWireDeclaration::Watch {
                port: 8463,
                token_dir: None,
            }),
            ..DaemonSettings::default()
        };
        let value = serde_json::to_value(&settings).expect("settings serialize");
        assert!(
            value.get(ROUTING_SETTINGS_KEY).is_some(),
            "no {ROUTING_SETTINGS_KEY} in {value}"
        );
    }

    /// The three answers `probe_routing` can give, read off the daemon's
    /// own constants.
    #[test]
    fn each_probe_answer_maps_to_its_own_outcome() {
        use trace_commons_contributor::daemon::ipc::{
            PROBE_REACHABLE, PROBE_TOKEN_UNREADABLE, PROBE_UNREACHABLE,
        };
        assert_eq!(
            parse_probe(&serde_json::json!({ "outcome": PROBE_REACHABLE })),
            ProbeOutcome::Reachable
        );
        assert_eq!(
            parse_probe(&serde_json::json!({
                "outcome": PROBE_TOKEN_UNREADABLE,
                "token_path": "/home/x/.ironwire/control.token",
            })),
            ProbeOutcome::TokenUnusable(Some("/home/x/.ironwire/control.token".to_string()))
        );
        // Absent, not null: the daemon omits the key when nothing
        // resolved at all, and unwrapping it here would panic.
        assert_eq!(
            parse_probe(&serde_json::json!({ "outcome": PROBE_TOKEN_UNREADABLE })),
            ProbeOutcome::TokenUnusable(None)
        );
        assert_eq!(
            parse_probe(&serde_json::json!({ "outcome": PROBE_UNREACHABLE, "port": 8463 })),
            ProbeOutcome::Unreachable(Some(8463))
        );
        // An answer this build cannot read claims nothing about the
        // proxy either way.
        assert_eq!(
            parse_probe(&serde_json::json!({ "outcome": "something_new" })),
            ProbeOutcome::Unknown
        );
        assert_eq!(parse_probe(&serde_json::json!({})), ProbeOutcome::Unknown);
    }

    /// Three outcomes, three sentences, none of them the same sentence.
    #[test]
    fn the_three_outcomes_never_repeat_a_sentence() {
        let lines = [
            probe_line(&ProbeOutcome::Reachable),
            probe_line(&ProbeOutcome::TokenUnusable(Some(
                "/home/x/.ironwire/control.token".to_string(),
            ))),
            probe_line(&ProbeOutcome::TokenUnusable(None)),
            probe_line(&ProbeOutcome::Unreachable(Some(8463))),
            probe_line(&ProbeOutcome::Unknown),
        ];
        for (i, a) in lines.iter().enumerate() {
            assert!(!a.is_empty(), "outcome {i} says nothing");
            for (j, b) in lines.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "outcomes {i} and {j} say the same thing");
                }
            }
        }
        // The one fact that fixes the failure a real contributor hits.
        assert!(
            lines[1].contains("/home/x/.ironwire/control.token"),
            "{}",
            lines[1]
        );
        assert!(lines[3].contains("8463"), "{}", lines[3]);
    }

    // --- What the machine already knows -------------------------------

    /// The method this shell calls is the one the daemon advertises.
    ///
    /// `discover_routing` sat in the daemon unused: its only references
    /// outside `daemon::ipc` were two doc comments and a list of names. A
    /// literal misspelled here would put it straight back to unused, and
    /// the failure would look exactly like a machine without IronWire.
    #[test]
    fn the_discovery_method_is_one_the_daemon_answers() {
        assert!(
            trace_commons_contributor::daemon::ipc::METHODS.contains(&DISCOVER_METHOD),
            "{DISCOVER_METHOD} is not a method the daemon advertises",
        );
    }

    /// The shape a running IronWire produces, and every shape that means
    /// there is nothing to offer.
    ///
    /// A machine without IronWire is the ordinary machine. Each of these
    /// reaches the same place -- no port -- and none of them is an error.
    #[test]
    fn only_a_published_port_is_something_to_offer() {
        assert_eq!(
            parse_discovery(&serde_json::json!({
                "found": true,
                "port": 9143,
                "token_path": "/home/x/.ironwire/control.token",
            })),
            Discovered { port: Some(9143) },
        );

        for (answer, why) in [
            (serde_json::json!({}), "an empty answer"),
            (
                serde_json::json!({ "found": false }),
                "the daemon's own no-pointer answer",
            ),
            (serde_json::json!({ "found": true }), "found with no port"),
            (
                serde_json::json!({ "found": true, "port": 0 }),
                "port zero, the ask-the-kernel sentinel",
            ),
            (
                serde_json::json!({ "found": true, "port": 70000 }),
                "a port above 65535",
            ),
            (
                serde_json::json!({ "found": true, "port": "8463" }),
                "a port that is not a number",
            ),
            (
                serde_json::json!({ "found": "true", "port": 8463 }),
                "found as a string",
            ),
        ] {
            assert_eq!(
                parse_discovery(&answer),
                Discovered::default(),
                "must offer nothing: {why}",
            );
        }
    }

    /// The rule the whole feature turns on, as a table.
    ///
    /// A declared port always wins. A pointer is a file that survives the
    /// daemon that wrote it, so a stale one naming 9000 must not replace a
    /// declared 8463 -- the same substitution `ironwire_ledger_for`
    /// refuses on the reading side.
    #[test]
    fn a_discovered_port_never_replaces_a_declared_one() {
        assert_eq!(shown_port(Some(8463), Some(9000)), 8463);
        assert_eq!(shown_port(Some(9000), None), 9000);
        // And it fills in where there is nothing declared, ahead of the
        // conventional number rather than behind it.
        assert_eq!(shown_port(None, Some(9143)), 9143);
        assert_eq!(shown_port(None, None), DEFAULT_IRONWIRE_PORT);
    }

    /// A shown port is still not a declaration, discovered or not.
    ///
    /// This is the same rule as `a_shown_default_is_not_a_declaration`,
    /// asserted for the number discovery supplies: putting it in the field
    /// writes nothing, and off is still spelled null.
    #[test]
    fn a_discovered_port_in_the_field_is_not_a_declaration() {
        let shown = shown_port(None, Some(9143));
        assert_eq!(shown, 9143);
        assert_eq!(routing_param(false, shown, ""), serde_json::Value::Null);
    }

    /// Discovery offers; it does not begin.
    ///
    /// Read from the source, because "this function does not call that
    /// one" is exactly the fact a later edit breaks silently and no
    /// value-level assertion can hold. The call that would turn an offer
    /// into a declaration is `set_settings`, and neither the ask nor the
    /// painter may reach it.
    #[test]
    fn discovery_writes_nothing_and_reads_nothing() {
        let source = include_str!("settings.rs");
        for name in ["fn discover_routing(", "fn render_discovery("] {
            let start = source.find(name).expect("the function exists");
            let end = source[start..].find("\n}\n").expect("its body ends") + start;
            let body = &source[start..end];
            for reached in [
                "set_settings",
                "routing_param",
                "send_routing",
                "probe_routing",
                "probe_routed_tools",
            ] {
                assert!(
                    !body.contains(reached),
                    "{name} reaches {reached}, which makes an offer into a declaration",
                );
            }
        }
    }

    /// Both sentences are the shared ones, and neither reads as a fault.
    #[test]
    fn the_discovery_sentence_is_the_shared_one_for_both_states() {
        let found = copy::ironwire_discovery_line(Some(9143));
        assert!(found.contains("9143"), "{found}");
        let nothing = copy::ironwire_discovery_line(None);
        assert_ne!(found, nothing);
        assert!(!nothing.contains("None"), "{nothing}");
        for line in [&found, &nothing] {
            let lower = line.to_lowercase();
            for word in ["error", "failed", "problem", "wrong"] {
                assert!(!lower.contains(word), "{word} in: {line}");
            }
        }
    }

    /// "Declared, nothing seen yet" is not a fault. A rebuilt ledger
    /// starts cold, so a contributor who just changed a setting sees this
    /// state -- and a window that painted it as a fault would accuse a
    /// working proxy of being broken at the moment they touched it.
    #[test]
    fn nothing_seen_yet_is_not_toned_as_a_fault() {
        use trace_commons_contributor::daemon::ipc::{
            ROUTING_AWAITING_ROWS, ROUTING_NOT_DECLARED, ROUTING_ROWS_SEEN,
        };
        let waiting = routing_tone(ROUTING_AWAITING_ROWS);
        assert_eq!(waiting, Tone::Held);
        assert_ne!(waiting, Tone::Attention);
        assert_ne!(waiting, Tone::Refused);
        assert_eq!(routing_tone(ROUTING_ROWS_SEEN), Tone::Clear);
        assert_eq!(routing_tone(ROUTING_NOT_DECLARED), Tone::Neutral);
        // A state this build cannot read is not a fault either.
        assert_eq!(routing_tone("something_new"), Tone::Neutral);
    }

    /// The probe is asked in the parameter names the daemon reads, and an
    /// empty folder box is left out rather than sent as an empty string,
    /// which `probe_routing` refuses outright.
    #[test]
    fn the_probe_is_asked_in_the_daemons_own_parameter_names() {
        let params = probe_params(8463, "/home/x/iw");
        assert_eq!(params["port"], serde_json::json!(8463));
        assert_eq!(params["token_dir"], serde_json::json!("/home/x/iw"));
        assert!(probe_params(8463, "  ").get("token_dir").is_none());
    }

    /// A tool-list answer, as the daemon sends one.
    fn answered(outcome: &str, tools: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "outcome": outcome, "tools": tools })
    }

    /// **Critical 1.** A stable failure stops the word asserting.
    ///
    /// The defect: a contributor whose proxy is dead read "Claude Code:
    /// Private" on the same card as "Nothing answered on port 8463". Both
    /// stable failures -- nothing listening, and a credential that is not
    /// there or is refused -- must reach `Unknown` however good the last
    /// answer was.
    #[test]
    fn a_proxy_that_stopped_answering_stops_the_word_asserting() {
        let listed = answered(
            "reachable",
            serde_json::json!([{ "id": "claude", "installed": true, "wired": true }]),
        );
        let alive = parse_routed_tools(&listed);
        assert_eq!(
            tool_wiring(Some(&alive), IRONWIRE_TOOL_CLAUDE),
            copy::ToolWiring::Wired
        );
        assert_eq!(
            copy::tool_word("watch", tool_wiring(Some(&alive), IRONWIRE_TOOL_CLAUDE)),
            copy::TOOL_PRIVATE
        );

        // Each failure carries the tool list a *previous* good answer had,
        // so the outcome is the only thing that can produce "not known".
        // Without that, the fixture would pass on an empty list alone and
        // the gate this asserts could be deleted unnoticed.
        for dead in [
            serde_json::json!({
                "outcome": "unreachable",
                "port": 8463,
                "tools": [{ "id": "claude", "installed": true, "wired": true }],
            }),
            serde_json::json!({
                "outcome": "token_unreadable",
                "token_path": "/x/control.token",
                "tools": [{ "id": "claude", "installed": true, "wired": true }],
            }),
        ] {
            let evidence = parse_routed_tools(&dead);
            assert_eq!(
                tool_wiring(Some(&evidence), IRONWIRE_TOOL_CLAUDE),
                copy::ToolWiring::Unknown,
                "{dead}"
            );
            let word = copy::tool_word("watch", tool_wiring(Some(&evidence), IRONWIRE_TOOL_CLAUDE));
            assert_eq!(word, copy::TOOL_UNKNOWN, "{dead}");
            assert_ne!(word, copy::TOOL_PRIVATE, "{dead}");
        }
    }

    /// The daemon state's tone is the shared table's, not a fourth copy.
    ///
    /// This was the last routing branch table written out natively in all
    /// three shells. Asserted on the source of the mapper, because three
    /// copies that agree today are what drift looks like the day before it
    /// happens -- and a value-level test would pass against a native table
    /// that still agreed.
    #[test]
    fn the_daemon_state_tone_is_not_reimplemented_in_this_shell() {
        use trace_commons_contributor::daemon::ipc::{
            ROUTING_AWAITING_ROWS, ROUTING_NOT_DECLARED, ROUTING_ROWS_SEEN,
        };
        let source = include_str!("settings.rs");
        let start = source
            .find("fn routing_tone(")
            .expect("the state tone mapper exists");
        let end = source[start..].find("\n}\n").expect("its body ends") + start;
        let body = &source[start..end];

        assert!(
            body.contains("copy::ironwire_state_tone(state)"),
            "the state tone must come from the shared branch table"
        );
        for spelled in [
            "ROUTING_AWAITING_ROWS",
            "ROUTING_ROWS_SEEN",
            "awaiting_rows",
            "rows_seen",
        ] {
            assert!(
                !body.contains(spelled),
                "the state tone is still branched on here: {spelled}"
            );
        }

        // And it carries the shared answer faithfully, over states this
        // build knows and ones it does not.
        for state in [
            ROUTING_NOT_DECLARED,
            ROUTING_AWAITING_ROWS,
            ROUTING_ROWS_SEEN,
            "",
            "a_state_from_a_later_daemon",
        ] {
            let expected = match copy::ironwire_state_tone(state) {
                copy::StateTone::Held => Tone::Held,
                copy::StateTone::Clear => Tone::Clear,
                copy::StateTone::Neutral => Tone::Neutral,
            };
            assert_eq!(routing_tone(state), expected, "{state}");
            // Neither of this palette's fault tones is reachable from any
            // state, including one this build has never heard of.
            assert_ne!(routing_tone(state), Tone::Attention, "{state}");
            assert_ne!(routing_tone(state), Tone::Refused, "{state}");
        }
    }

    /// No styling decision on a tool row reads the rendered word.
    ///
    /// The chip's tone comes from [`copy::tool_tone`], which takes what the
    /// word takes. Asserted on the source of the one painter, because "this
    /// does not compare a string" is a fact a later edit reintroduces
    /// silently: `Private` is a substring of the denial that must never come
    /// back, and a `contains` against a privacy claim is the same shape that
    /// once matched `"unreachable"` as `"reachable"` on this surface.
    #[test]
    fn no_tool_row_styling_decision_reads_the_rendered_word() {
        let source = include_str!("settings.rs");
        let start = source
            .find("fn render_tool_rows(")
            .expect("the row painter exists");
        let end = source[start..].find("\n}\n").expect("its body ends") + start;
        let body = &source[start..end];

        assert!(
            body.contains("copy::tool_tone(mode, wiring)"),
            "the row painter must take its tone from the shared branch table"
        );
        for comparison in [
            "TOOL_PRIVATE",
            "word ==",
            "word !=",
            "word.contains",
            "word.starts_with",
            "word.eq",
        ] {
            assert!(
                !body.contains(comparison),
                "a styling decision reads the rendered word: {comparison}"
            );
        }

        // And the tone is the one the shared table chose, over every pair.
        for mode in ["off", "watch", "unset", ""] {
            for wiring in [
                copy::ToolWiring::Wired,
                copy::ToolWiring::NotWired,
                copy::ToolWiring::Unknown,
            ] {
                let clear = copy::tool_tone(mode, wiring) == copy::ToolTone::Clear;
                assert_eq!(
                    clear,
                    copy::tool_word(mode, wiring) == copy::TOOL_PRIVATE,
                    "{mode:?}/{wiring:?}"
                );
            }
        }
    }

    /// **Critical 1, the other half.** `awaiting_rows` must not downgrade.
    ///
    /// It is the daemon's own state and is not an input to `tool_wiring` at
    /// all. A proxy installed this morning reports it, and a declaration
    /// change puts a working install back into it, so a word that fell to
    /// "not known" on it would flicker against a machine where nothing is
    /// wrong. Asserted through the tone function, which is where that state
    /// is read, and by construction: the word is computed from the same
    /// evidence either way.
    #[test]
    fn awaiting_rows_does_not_downgrade_the_word() {
        use trace_commons_contributor::daemon::ipc::{ROUTING_AWAITING_ROWS, ROUTING_ROWS_SEEN};
        let evidence = parse_routed_tools(&answered(
            "reachable",
            serde_json::json!([{ "id": "claude", "installed": true, "wired": true }]),
        ));
        let word = copy::tool_word("watch", tool_wiring(Some(&evidence), IRONWIRE_TOOL_CLAUDE));
        assert_eq!(word, copy::TOOL_PRIVATE);
        // The state the daemon would be reporting alongside it, in either
        // of its two "on" values, changes nothing about that word.
        assert_ne!(routing_tone(ROUTING_AWAITING_ROWS), Tone::Attention);
        assert_ne!(routing_tone(ROUTING_ROWS_SEEN), Tone::Attention);

        // And it cannot, structurally: the daemon's state reaches only
        // `render_routing_status`, which paints the status block and never
        // the tool rows. Read from the source, because "this function does
        // not call that one" is the kind of fact a later edit breaks
        // silently and no value-level assertion can hold.
        let source = include_str!("settings.rs");
        let start = source
            .find("fn render_routing_status(")
            .expect("the status painter exists");
        let end = source[start..].find("\n}\n").expect("its body ends") + start;
        let body = &source[start..end];
        for reached in [
            "render_tool_rows",
            "routing_tools",
            "tool_wiring",
            "tool_word",
        ] {
            assert!(
                !body.contains(reached),
                "the daemon state painter must not reach {reached}"
            );
        }
        // The word's own input takes no daemon state at all.
        assert!(
            source.contains(
                "fn tool_wiring(evidence: Option<&RoutingEvidence>, id: &str) -> copy::ToolWiring"
            ),
            "tool_wiring must take evidence and a tool id, and nothing else"
        );
    }

    /// **Critical 2.** The word is per tool, not per switch.
    ///
    /// One declaration, one answer, three different words -- which is
    /// exactly what the old `settings.ironwire.mode == "watch"` input could
    /// not produce. Gemini CLI is the case that made it wrong: IronWire has
    /// no row for it on any machine today, so the only honest word is "not
    /// known", and the old code printed "Private".
    #[test]
    fn one_declaration_produces_three_different_words() {
        let evidence = parse_routed_tools(&answered(
            "reachable",
            serde_json::json!([
                { "id": "claude", "installed": true, "wired": true },
                { "id": "codex", "installed": true, "wired": false },
            ]),
        ));
        let claude = copy::tool_word("watch", tool_wiring(Some(&evidence), IRONWIRE_TOOL_CLAUDE));
        let codex = copy::tool_word("watch", tool_wiring(Some(&evidence), IRONWIRE_TOOL_CODEX));
        let gemini = copy::tool_word("watch", tool_wiring(Some(&evidence), IRONWIRE_TOOL_GEMINI));
        let cline = copy::tool_word("watch", tool_wiring(Some(&evidence), IRONWIRE_TOOL_CLINE));
        assert_eq!(claude, copy::TOOL_PRIVATE);
        assert_eq!(codex, copy::TOOL_DIRECT);
        assert_eq!(gemini, copy::TOOL_UNKNOWN);
        // Same shape as Gemini: no upstream row, so no verdict, however the
        // declaration switch is set.
        assert_eq!(cline, copy::TOOL_UNKNOWN);
        assert_ne!(claude, codex);
        assert_ne!(codex, gemini);
    }

    /// Nothing has been asked yet, so nothing may be claimed.
    #[test]
    fn no_answer_yet_is_not_a_verdict() {
        assert_eq!(
            tool_wiring(None, IRONWIRE_TOOL_CLAUDE),
            copy::ToolWiring::Unknown
        );
        assert_eq!(
            copy::tool_word("watch", tool_wiring(None, IRONWIRE_TOOL_CLAUDE)),
            copy::TOOL_UNKNOWN
        );
        // A tool the contributor does not use is still "not used": that
        // answer never needed evidence.
        assert_eq!(
            copy::tool_word("off", tool_wiring(None, IRONWIRE_TOOL_CLAUDE)),
            copy::TOOL_NOT_USED
        );
    }

    /// Two detectors disagreeing about one machine is not evidence.
    ///
    /// IronWire saying a tool is not present, while this app is watching
    /// that tool's sessions, gets no verdict in either direction.
    #[test]
    fn a_tool_ironwire_says_is_absent_gets_no_verdict() {
        let evidence = parse_routed_tools(&answered(
            "reachable",
            serde_json::json!([{ "id": "codex", "installed": false, "wired": false }]),
        ));
        assert_eq!(
            tool_wiring(Some(&evidence), IRONWIRE_TOOL_CODEX),
            copy::ToolWiring::Unknown
        );
    }

    /// A missing field is never read as a claim.
    #[test]
    fn an_answer_missing_its_fields_claims_nothing() {
        let evidence = parse_routed_tools(&answered(
            "reachable",
            serde_json::json!([{ "id": "claude" }, { "wired": true }]),
        ));
        assert_eq!(evidence.tools.len(), 1, "a row without an id is not a row");
        assert_eq!(
            tool_wiring(Some(&evidence), IRONWIRE_TOOL_CLAUDE),
            copy::ToolWiring::Unknown
        );
        // An outcome this build cannot read claims nothing either.
        let strange = parse_routed_tools(&answered(
            "something_new",
            serde_json::json!([{ "id": "claude", "installed": true, "wired": true }]),
        ));
        assert_eq!(strange.outcome, ProbeOutcome::Unknown);
        assert_eq!(
            tool_wiring(Some(&strange), IRONWIRE_TOOL_CLAUDE),
            copy::ToolWiring::Unknown
        );
    }

    /// The backstop is a multiple of the render cadence it backs.
    ///
    /// Both halves are read rather than restated: the daemon's default
    /// `poll_interval_secs` is what sets how often a daemon event re-renders
    /// this card, and a backstop at or below that would re-ask on every
    /// tick forever rather than backstopping anything. Four ticks is the
    /// floor asserted; the constant is five.
    #[test]
    fn the_backstop_is_a_multiple_of_the_poll_it_backs() {
        let poll = std::time::Duration::from_secs(
            trace_commons_contributor::daemon::settings::DaemonSettings::default()
                .poll_interval_secs,
        );
        assert_eq!(poll, std::time::Duration::from_secs(60), "the poll moved");
        assert!(
            EVIDENCE_BACKSTOP_TTL >= poll * 4,
            "a backstop at {EVIDENCE_BACKSTOP_TTL:?} against a {poll:?} poll re-asks every tick"
        );
        // And not so long that a card left open never notices a proxy that
        // went away. Half an hour is already far past useful.
        assert!(EVIDENCE_BACKSTOP_TTL <= std::time::Duration::from_secs(30 * 60));
    }

    /// A re-ask does not blank the word.
    ///
    /// `ask_routed_tools` reads the cache to decide whether to call and
    /// never clears it, so an expiry is invisible on screen unless the
    /// answer that comes back actually differs. Read from the source,
    /// because "this function does not clear that cache" is not a
    /// value-level assertion and is exactly what a later edit breaks.
    #[test]
    fn expiring_the_evidence_does_not_flicker_the_word() {
        let source = include_str!("settings.rs");
        let start = source
            .find("fn ask_routed_tools(")
            .expect("the asker exists");
        let end = source[start..].find("\n}\n").expect("its body ends") + start;
        let body = &source[start..end];
        assert!(
            body.contains("EVIDENCE_BACKSTOP_TTL"),
            "the asker must be the thing that reads the backstop"
        );
        assert!(
            !body.contains("routing_evidence.replace(None)"),
            "re-asking must not clear the answer that is on screen"
        );
    }

    /// The ids are IronWire's own, which is what the response is keyed by.
    #[test]
    fn the_tool_ids_are_the_ones_ironwire_answers_with() {
        assert_eq!(IRONWIRE_TOOL_CLAUDE, "claude");
        assert_eq!(IRONWIRE_TOOL_CODEX, "codex");
        assert_eq!(IRONWIRE_TOOL_GEMINI, "gemini");
        assert_eq!(IRONWIRE_TOOL_CLINE, "cline");
    }
}
