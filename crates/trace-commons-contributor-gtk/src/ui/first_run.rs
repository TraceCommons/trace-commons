//! GTK first run, ported from the Mac's (`OnboardingCoordinatorView`,
//! `FirstRunRunner`, the screens under `Views/FirstRun`). Answers stay
//! local until a commit: leaving Tools and Customize send the planner's
//! `Commit::Roots`, Sign in on Join sends `Commit::SignIn`, and Start
//! sharing sends `Commit::Start`. Every decision is `crate::first_run`;
//! this file draws it and makes the calls.
//!
//! The stage is dark whatever the desktop uses, as the Mac forces its
//! first-run stage dark (`OnboardingCoordinatorView`): see [`super::stage`].
use super::{
    App,
    first_run_screens::{self as screens, fill, page},
    first_run_terms::{Accepted, TermsPage},
    first_run_tools::{self, ToolsScreen},
    onboarding::body_label,
};
use crate::{
    first_run::{
        Account, Action, Commit, Outcome, Rule, Runner, Sharing, State, Step, Ticket, join, org,
        rules,
        tools::Tools,
        uses::{self, ScopeOption},
    },
    model::Project,
};
use adw::prelude::*;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
use trace_commons_contributor::{consent_copy, first_run_copy::first_run_copy, flow1};

type DeferredRead = Box<dyn FnOnce(&Rc<Coordinator>)>;
type Done = Box<dyn FnOnce(&Rc<Coordinator>, Option<String>)>;

#[derive(Default)]
struct ReadRequests {
    active: std::collections::BTreeMap<String, u64>,
    pending: std::collections::BTreeMap<String, (u64, DeferredRead)>,
}

pub(super) use super::stage::StageScheme;

pub(super) struct Coordinator {
    app: Weak<App>,
    window: glib::WeakRef<adw::Window>,
    stack: gtk::Stack,
    heading: adw::WindowTitle,
    state: RefCell<State>,
    tools: Rc<RefCell<Tools>>,
    runner: RefCell<Runner>,
    done: RefCell<Option<Done>>,
    accepted: RefCell<Option<Rc<RefCell<Option<Accepted>>>>>,
    error: gtk::Label,
    back: gtk::Button,
    busy: Cell<bool>,
    closed: Cell<bool>,
    generation: Cell<u64>,
    reads: RefCell<ReadRequests>,
    tools_screen: RefCell<Option<Rc<ToolsScreen>>>,
    private_body: gtk::Box,
    traces_body: gtk::Box,
    trace_lines: RefCell<Option<Value>>,
    trace_lines_asked: Cell<bool>,
    options: RefCell<Vec<ScopeOption>>,
    uses_card: gtk::Box,
    uses_flow: gtk::Box,
    uses_action: gtk::Button,
    customize: gtk::Button,
    customize_box: gtk::Box,
    customizing: Cell<bool>,
    join_body: gtk::Box,
    start: gtk::Button,
    login_attempt: RefCell<Option<String>>,
    login_waiting: Cell<bool>,
    invite_draft: RefCell<Option<String>>,
    invite_refused: Cell<bool>,
    invite_dead: Cell<bool>,
    org_checked: Cell<bool>,
    auto_started: Cell<bool>,
    /// Whether a signed-in NEAR AI account was held when Join last drew.
    forward: Cell<bool>,
    /// The daemon's answer for the invite looked up, for its pay range.
    lookup: RefCell<Option<Value>>,
    /// The optional referral code under near.ai's Sign in, sent with the
    /// NEAR AI enrollment.
    referral: RefCell<String>,
    /// The NEAR AI credit section, drawn once an enrollment is held.
    payout: RefCell<Option<Rc<super::payout_link::PayoutLinkView>>>,
    payout_shown: Cell<bool>,
    /// The animated stage under every step (`FirstRunStage.swift`).
    stage: Rc<super::first_run_stage_view::Stage>,
}

fn name(step: Step) -> &'static str {
    match step {
        Step::Welcome => "welcome",
        Step::Tools => "tools",
        Step::PrivateAi => "private-ai",
        Step::Traces => "traces",
        Step::Uses => "uses",
        Step::Terms => "terms",
        Step::Join => "join",
    }
}
fn title(a: &str, b: &str) -> String {
    [a, b].concat()
}
fn primary(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("suggested-action");
    button.set_halign(gtk::Align::End);
    button
}
fn clear(root: &gtk::Box) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
}
fn eyebrow(text: &str) -> gtk::Label {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    label.add_css_class("tc-eyebrow");
    label
}
/// An "i" beside a row, its words in a popover: the Mac's info button.
fn info(title: &str, text: &str) -> gtk::MenuButton {
    let popover = gtk::Popover::new();
    let label = body_label(text);
    label.set_max_width_chars(48);
    label.set_margin_start(8);
    label.set_margin_end(8);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    popover.set_child(Some(&label));
    let button = gtk::MenuButton::builder()
        .icon_name("dialog-information-symbolic")
        .popover(&popover)
        .build();
    button.add_css_class("flat");
    let name = fill(first_run_copy().uses.more_about, "title", title);
    button.set_tooltip_text(Some(&name));
    button.update_property(&[gtk::accessible::Property::Label(&name)]);
    button
}

pub(super) fn present(app: &Rc<App>) -> adw::Window {
    super::style::install();
    super::community_brand::install();
    let copy = first_run_copy();
    let window = adw::Window::builder()
        .transient_for(&app.window)
        .modal(true)
        .default_width(680)
        .default_height(720)
        .build();
    StageScheme::hold(&window);
    // The roots window's handoff ends once this window holds the stage.
    StageScheme::end_handoff();
    let stack = gtk::Stack::builder()
        .vexpand(true)
        .vhomogeneous(false)
        // Steps fade into each other, as the Mac's do, so the stage's N and
        // tiles carry the move between them.
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let heading = adw::WindowTitle::new(crate::copy::APP_NAME, "");
    let error = body_label("");
    error.add_css_class("tc-refused");
    error.set_visible(false);
    let back = gtk::Button::with_label(copy.frame.back);
    let start = primary(copy.uses.start);
    start.set_sensitive(false);
    let tools = Rc::new(RefCell::new(Tools::default()));
    let stage = super::first_run_stage_view::Stage::new(tools.clone());
    let c = Rc::new(Coordinator {
        app: Rc::downgrade(app),
        window: window.downgrade(),
        stack: stack.clone(),
        heading: heading.clone(),
        state: RefCell::new(State {
            daemon_started: true,
            ..State::default()
        }),
        tools,
        runner: RefCell::new(Runner::default()),
        done: RefCell::new(None),
        accepted: RefCell::new(None),
        error: error.clone(),
        back: back.clone(),
        busy: Cell::new(false),
        closed: Cell::new(false),
        generation: Cell::new(0),
        reads: RefCell::new(ReadRequests::default()),
        tools_screen: RefCell::new(None),
        private_body: gtk::Box::new(gtk::Orientation::Vertical, 12),
        traces_body: gtk::Box::new(gtk::Orientation::Vertical, 12),
        trace_lines: RefCell::new(None),
        trace_lines_asked: Cell::new(false),
        options: RefCell::new(Vec::new()),
        uses_card: gtk::Box::new(gtk::Orientation::Vertical, 12),
        uses_flow: gtk::Box::new(gtk::Orientation::Vertical, 0),
        uses_action: primary(copy.join.skip),
        customize: gtk::Button::with_label(copy.frame.custom_setup_instead),
        customize_box: gtk::Box::new(gtk::Orientation::Vertical, 12),
        customizing: Cell::new(false),
        join_body: gtk::Box::new(gtk::Orientation::Vertical, 16),
        start: start.clone(),
        login_attempt: RefCell::new(None),
        login_waiting: Cell::new(false),
        invite_draft: RefCell::new(None),
        invite_refused: Cell::new(false),
        invite_dead: Cell::new(false),
        org_checked: Cell::new(false),
        auto_started: Cell::new(false),
        forward: Cell::new(false),
        lookup: RefCell::new(None),
        referral: RefCell::new(String::new()),
        payout: RefCell::new(None),
        payout_shown: Cell::new(false),
        stage: stage.clone(),
    });
    if let Some(handed) = super::roots::take_first_run_handoff() {
        let mut state = c.state.borrow_mut();
        *c.tools.borrow_mut() = handed.clone();
        state.tools = handed;
        state.sync_roots();
        state.committed_roots = Some(state.roots.clone());
        state.step = Step::PrivateAi;
    }
    c.build_pages(&window);
    back.connect_clicked({
        let weak = Rc::downgrade(&c);
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.go_back();
            }
        }
    });
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    content.add_css_class("tc-first-run-stage");
    content.append(&adw::HeaderBar::builder().title_widget(&heading).build());
    content.append(stage.thread());
    // Every step the same width, as the Mac's frame holds its content to
    // one column (`FirstRunFrame`).
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&super::first_run_screens::column(&stack))
        .build();
    stage.follow(&scroll);
    content.append(&scroll);
    // Back at the bar's left, as on the Mac (`FirstRunFrame.actionBar`).
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    footer.set_margin_start(24);
    footer.set_margin_end(24);
    footer.set_margin_top(12);
    footer.set_margin_bottom(16);
    error.set_hexpand(true);
    back.set_halign(gtk::Align::Start);
    footer.append(&back);
    footer.append(&error);
    content.append(&footer);
    window.set_content(Some(&stage.wrap(&content)));
    window.connect_close_request({
        let c = c.clone();
        move |_| {
            c.closed.set(true);
            c.generation.set(c.generation.get().wrapping_add(1));
            c.cancel_login();
            c.runner.borrow_mut().cancel();
            glib::Propagation::Proceed
        }
    });
    c.show();
    window.present();
    window
}

impl Coordinator {
    fn build_pages(self: &Rc<Self>, window: &adw::Window) {
        let copy = first_run_copy();
        let weak = Rc::downgrade(self);
        let next = {
            let weak = weak.clone();
            move || {
                if let Some(c) = weak.upgrade() {
                    c.next();
                }
            }
        };
        let welcome = screens::welcome(next.clone(), {
            let weak = weak.clone();
            move || {
                if let Some(c) = weak.upgrade() {
                    c.stage.set_agent(true);
                    c.stack.set_visible_child_name("agent");
                }
            }
        });
        welcome.prepend(&self.stage.slot(Step::Welcome));
        self.stack.add_named(&welcome, Some("welcome"));
        let agent = screens::agent_configuration({
            let weak = weak.clone();
            move || {
                if let Some(c) = weak.upgrade() {
                    c.stage.set_agent(false);
                    c.stack.set_visible_child_name("welcome");
                }
            }
        });
        self.stack.add_named(&agent, Some("agent"));
        let tools = first_run_tools::build(self.tools.clone(), window.upcast_ref(), {
            let weak = weak.clone();
            move || {
                if let Some(c) = weak.upgrade() {
                    c.leave_tools();
                }
            }
        });
        tools.root.prepend(&self.stage.slot(Step::Tools));
        self.stack.add_named(&tools.root, Some("tools"));
        *self.tools_screen.borrow_mut() = Some(tools);

        let private_ai = page(&title(
            copy.private_ai.title_light,
            copy.private_ai.title_bold,
        ));
        private_ai.prepend(&self.stage.slot(Step::PrivateAi));
        private_ai.append(&self.private_body);
        let button = primary(copy.frame.continue_button);
        button.connect_clicked({
            let next = next.clone();
            move |_| next()
        });
        private_ai.append(&button);
        self.stack.add_named(&private_ai, Some("private-ai"));

        let traces = page(&title(copy.traces.title_light, copy.traces.title_bold));
        traces.prepend(&self.stage.slot(Step::Traces));
        traces.append(&self.traces_body);
        let button = primary(copy.frame.continue_button);
        button.connect_clicked({
            let next = next.clone();
            move |_| next()
        });
        traces.append(&button);
        self.stack.add_named(&traces, Some("traces"));

        let uses_page = page(&title(copy.uses.title_light, copy.uses.title_bold));
        // The flow's captions sit under the stage's graphic, above the
        // title, as the Mac sets them inside its graphic.
        let uses_slot = self.stage.slot(Step::Uses);
        uses_page.prepend(&uses_slot);
        uses_page.insert_child_after(&self.uses_flow, Some(&uses_slot));
        uses_page.append(&self.uses_card);
        self.customize_box.set_visible(false);
        uses_page.append(&self.customize_box);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        self.customize.add_css_class("flat");
        self.customize.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(c) = weak.upgrade() {
                    c.open_customize();
                }
            }
        });
        self.uses_action.set_hexpand(true);
        self.uses_action.connect_clicked({
            let next = next.clone();
            move |_| next()
        });
        actions.append(&self.customize);
        actions.append(&self.uses_action);
        uses_page.append(&actions);
        self.stack.add_named(&uses_page, Some("uses"));

        let terms = TermsPage::new(next.clone());
        *self.accepted.borrow_mut() = Some(terms.accepted);
        terms.root.prepend(&self.stage.slot(Step::Terms));
        self.stack.add_named(&terms.root, Some("terms"));

        let join_page = page(&title(copy.join.title_light, copy.join.title_bold));
        join_page.prepend(&self.stage.slot(Step::Join));
        join_page.append(&body_label(copy.join.body));
        join_page.append(&self.join_body);
        let note = body_label(copy.join.no_sharing);
        note.add_css_class("tc-meta");
        join_page.append(&note);
        join_page.append(&self.start);
        self.start.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(c) = weak.upgrade() {
                    c.start();
                }
            }
        });
        self.stack.add_named(&join_page, Some("join"));
    }

    fn show(self: &Rc<Self>) {
        self.generation.set(self.generation.get().wrapping_add(1));
        let step = self.state.borrow().step;
        let copy = first_run_copy();
        // Welcome names no step, as on the Mac: its own title is on the page.
        self.heading.set_subtitle(match step {
            Step::Welcome => "",
            Step::Tools => copy.frame.step_tools,
            Step::PrivateAi => copy.frame.step_private_ai,
            Step::Traces => copy.frame.step_traces,
            Step::Uses => copy.frame.step_uses,
            Step::Terms => copy.frame.step_terms,
            Step::Join => copy.frame.step_join,
        });
        self.stack.set_visible_child_name(name(step));
        self.stage.set_agent(false);
        self.stage.set_step(step, !self.busy.get(), {
            let weak = Rc::downgrade(self);
            Rc::new(move |picked| {
                if let Some(c) = weak.upgrade() {
                    if !c.busy.get() {
                        if c.customizing.get() {
                            c.close_customize();
                        }
                        c.state.borrow_mut().back_to(picked);
                        c.show();
                    }
                }
            })
        });
        self.back.set_visible(step != Step::Welcome);
        self.error.set_visible(false);
        match step {
            Step::PrivateAi => self.render_private_ai(),
            Step::Traces => self.render_traces(),
            Step::Uses => {
                self.close_customize();
                self.render_uses();
                self.load_scopes();
            }
            Step::Join => self.load_join(),
            Step::Welcome | Step::Tools | Step::Terms => {}
        }
    }
    fn next(self: &Rc<Self>) {
        if !self.busy.get() {
            self.state.borrow_mut().next();
            self.show();
        }
    }
    fn go_back(self: &Rc<Self>) {
        if self.customizing.get() {
            self.close_customize();
            return;
        }
        if self.login_waiting.get() {
            // As the Mac's Back during the browser wait: end the wait (nothing
            // enrolled, nothing said), then go back a step.
            self.cancel_login();
            self.state.borrow_mut().back();
            self.show();
            return;
        }
        if !self.busy.get() {
            self.state.borrow_mut().back();
            self.show();
        }
    }
    fn fail(&self, message: &str) {
        self.error.set_label(message);
        self.error.set_visible(true);
        self.set_busy(false);
    }
    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.stack.set_sensitive(!busy || self.login_waiting.get());
        self.back.set_sensitive(!busy || self.login_waiting.get());
    }
    fn receipt(&self) -> Option<Accepted> {
        self.accepted
            .borrow()
            .as_ref()
            .and_then(|r| r.borrow().clone())
    }
    fn valid(&self, generation: u64) -> bool {
        !self.closed.get() && self.generation.get() == generation
    }
    /// What Tools answered, into the state the planner reads.
    fn sync_tools(&self) {
        let mut state = self.state.borrow_mut();
        state.tools = self.tools.borrow().clone();
        state.sync_roots();
    }
    fn required(&self) -> Option<String> {
        uses::required_scope(&self.options.borrow()).map(|o| o.name.clone())
    }
    // Coalesce repeated requests for the same view. If navigation invalidates
    // an in-flight request, keep just the latest reload until it completes.
    fn begin_read(&self, key: &str, step: Step, reload: DeferredRead) -> Option<u64> {
        if self.closed.get() || self.state.borrow().step != step {
            return None;
        }
        let generation = self.generation.get();
        let mut reads = self.reads.borrow_mut();
        if let Some(active) = reads.active.get(key) {
            if *active != generation {
                reads.pending.insert(key.to_owned(), (generation, reload));
            }
            return None;
        }
        reads.active.insert(key.to_owned(), generation);
        Some(generation)
    }
    fn finish_read(self: &Rc<Self>, key: &str, generation: u64) -> bool {
        let pending = {
            let mut reads = self.reads.borrow_mut();
            if reads.active.get(key) != Some(&generation) {
                return false;
            }
            reads.active.remove(key);
            reads.pending.remove(key)
        };
        if let Some((queued_generation, reload)) = pending {
            if self.valid(queued_generation) {
                reload(self);
            }
        }
        self.valid(generation)
    }

    // ---- Tools ---------------------------------------------------------

    /// Continue on Tools: the planner's `Commit::Roots`, then the next step.
    fn leave_tools(self: &Rc<Self>) {
        self.sync_tools();
        self.commit(
            Commit::Roots,
            Box::new(|c, failure| {
                if let Some(screen) = c.tools_screen.borrow().as_ref() {
                    screen.finish(failure.as_deref());
                }
                if failure.is_none() {
                    c.next();
                }
            }),
        );
    }

    // ---- Private AI and Traces -----------------------------------------

    fn render_private_ai(&self) {
        clear(&self.private_body);
        let tools = self.tools.borrow();
        self.private_body.append(&screens::private_ai_body(
            &tools.connectable(),
            tools.found_count(),
        ));
    }

    fn render_traces(self: &Rc<Self>) {
        clear(&self.traces_body);
        let copy = first_run_copy();
        let (found, bytes) = {
            let tools = self.tools.borrow();
            (tools.trace_count(), tools.total_bytes())
        };
        if let Some(count) = found {
            // The Mac's count over "traces on this Mac", for any computer
            // (owner, 2026-11-17).
            let figure = gtk::Label::builder()
                .label(count.to_string())
                .xalign(0.0)
                .build();
            figure.add_css_class("tc-flow-title");
            let caption = body_label(copy.traces.on_this_computer);
            caption.add_css_class("tc-meta");
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.append(&figure);
            column.append(&caption);
            column.update_property(&[gtk::accessible::Property::Label(&format!(
                "{count} {}",
                copy.traces.on_this_computer
            ))]);
            self.traces_body.append(&column);
        }
        let earns = self
            .state
            .borrow()
            .scopes
            .iter()
            .any(|s| s == copy.uses.credit_scope);
        if let Some((headline, note)) =
            screens::traces_lines(found, self.trace_lines.borrow().as_ref(), earns)
        {
            self.traces_body.append(&body_label(&headline));
            if let Some(note) = note {
                let note = body_label(&note);
                note.add_css_class("tc-meta");
                self.traces_body.append(&note);
            }
        }
        // Asked once per first run, once something is counted.
        if found.is_some_and(|n| n > 0) && !self.trace_lines_asked.replace(true) {
            let Some(app) = self.app.upgrade() else {
                return;
            };
            let weak = Rc::downgrade(self);
            app.call(
                "first_run_copy",
                json!({"trace_count": found, "total_bytes": bytes}),
                move |_, result| {
                    if let (Some(c), Ok(value)) = (weak.upgrade(), result) {
                        *c.trace_lines.borrow_mut() = Some(value);
                        if c.state.borrow().step == Step::Traces {
                            c.render_traces();
                        }
                    }
                },
            );
        }
    }

    // ---- Uses ----------------------------------------------------------

    fn load_scopes(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(generation) = self.begin_read("scopes", Step::Uses, Box::new(|c| c.load_scopes()))
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        app.call("consent_options", json!({}), move |_, result| {
            let Some(c) = weak
                .upgrade()
                .filter(|c| c.finish_read("scopes", generation))
            else {
                return;
            };
            match parse_options(result) {
                Some(options) => {
                    *c.options.borrow_mut() = options;
                    let required = c.required();
                    uses::include_required(&mut c.state.borrow_mut(), required.as_deref());
                    c.render_uses();
                }
                None => c.fail(first_run_copy().uses.scopes_failed),
            }
        });
    }

    fn render_uses(self: &Rc<Self>) {
        let copy = first_run_copy();
        let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
        let grant = consent_copy::automatic_contribution_copy(None);
        let options = self.options.borrow().clone();
        let state = self.state.borrow().clone();
        clear(&self.uses_flow);
        self.stage.set_sharing(state.sharing);
        let gate = if state.sharing_chosen && state.sharing == Sharing::Automatic {
            modes.choices[1].label
        } else {
            copy.uses.you_review
        };
        self.uses_flow.append(&screens::uses_flow(gate));
        clear(&self.uses_card);
        for option in uses::shown_optional_scopes(&options, copy.uses.credit_scope) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let check = gtk::CheckButton::with_label(&option.title);
            check.set_active(state.scopes.contains(&option.name));
            let key = option.name.clone();
            check.connect_toggled({
                let weak = Rc::downgrade(self);
                move |check| {
                    if let Some(c) = weak.upgrade() {
                        {
                            let mut state = c.state.borrow_mut();
                            state.scopes.retain(|s| s != &key);
                            if check.is_active() {
                                state.scopes.push(key.clone());
                            }
                            state.grant_ready = false;
                        }
                        c.update_uses_action();
                    }
                }
            });
            column.append(&check);
            if option.name == copy.uses.credit_scope {
                let warning = body_label(copy.uses.credit_required);
                warning.add_css_class("tc-attention");
                column.append(&warning);
            }
            row.append(&column);
            row.append(&info(&option.title, &option.description));
            self.uses_card.append(&row);
        }
        if let Some(required) = uses::required_scope(&options) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let line = body_label(&fill(copy.uses.always_on, "title", &required.title));
            line.add_css_class("tc-meta");
            row.append(&line);
            row.append(&info(&required.title, &required.description));
            self.uses_card.append(&row);
        }
        self.uses_card
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        heading.append(&eyebrow(copy.uses.sharing));
        let detail = if state.sharing == Sharing::Automatic {
            grant
                .patterns_only
                .or(grant.model_scrubbed)
                .map(|s| [copy.uses.automatic_scope, s.scope, s.limit].join(" "))
        } else {
            Some(grant.path_ask_first_detail.to_owned()).filter(|d| !d.is_empty())
        };
        if let Some(detail) = detail {
            heading.append(&info(copy.uses.sharing, &detail));
        }
        self.uses_card.append(&heading);
        let ask = gtk::CheckButton::with_label(copy.uses.ask_each_time);
        let automatic = gtk::CheckButton::with_label(modes.choices[1].label);
        automatic.set_group(Some(&ask));
        // Neither is chosen for the person (owner, 2026-10-27).
        match uses::sharing_selection(&state) {
            Some(Sharing::Ask) => ask.set_active(true),
            Some(Sharing::Automatic) => automatic.set_active(true),
            None => {}
        }
        self.uses_card.append(&ask);
        if !grant.path_ask_first_title.is_empty() {
            let line = body_label(grant.path_ask_first_title);
            line.add_css_class("tc-meta");
            self.uses_card.append(&line);
        }
        self.uses_card.append(&automatic);
        let mut lines = vec![copy.uses.automatic_scope];
        if !state.holds_enrollment() {
            lines.push(copy.uses.automatic_after_sign_in);
        }
        let line = body_label(&lines.join("\n"));
        line.add_css_class("tc-meta");
        self.uses_card.append(&line);
        for (button, sharing) in [(ask, Sharing::Ask), (automatic, Sharing::Automatic)] {
            button.connect_toggled({
                let weak = Rc::downgrade(self);
                move |button| {
                    if !button.is_active() {
                        return;
                    }
                    if let Some(c) = weak.upgrade() {
                        uses::choose_sharing(&mut c.state.borrow_mut(), sharing);
                        let weak = Rc::downgrade(&c);
                        glib::idle_add_local_once(move || {
                            if let Some(c) = weak.upgrade() {
                                c.render_uses();
                            }
                        });
                    }
                }
            });
        }
        self.update_uses_action();
    }

    fn update_uses_action(&self) {
        let copy = first_run_copy();
        let state = self.state.borrow();
        let options = self.options.borrow();
        self.uses_action
            .set_label(if uses::has_selection(&state, &options) {
                copy.frame.continue_button
            } else {
                copy.join.skip
            });
        self.uses_action.set_sensitive(
            state.step != Step::Uses || state.can_continue(self.required().is_some()),
        );
        self.customize.set_visible(!self.customizing.get());
        self.customize
            .set_sensitive(!self.busy.get() && self.tools.borrow().watches_anything());
    }

    /// Customize opens the per-repo rules in place. The daemon is told the
    /// tools ticked first, so it can name the repos found in their traces.
    fn open_customize(self: &Rc<Self>) {
        if self.busy.get() || !self.tools.borrow().watches_anything() {
            return;
        }
        self.customizing.set(true);
        self.stage.set_open(true);
        self.uses_card.set_visible(false);
        self.uses_flow.set_visible(false);
        self.customize_box.set_visible(true);
        self.update_uses_action();
        clear(&self.customize_box);
        let spinner = gtk::Spinner::new();
        spinner.start();
        self.customize_box.append(&spinner);
        self.sync_tools();
        self.commit(
            Commit::Roots,
            Box::new(|c, failure| match failure {
                Some(line) => c.fail(&line),
                None => c.load_projects(),
            }),
        );
    }

    fn close_customize(self: &Rc<Self>) {
        self.customizing.set(false);
        self.stage.set_open(false);
        self.uses_card.set_visible(true);
        self.uses_flow.set_visible(true);
        self.customize_box.set_visible(false);
        self.update_uses_action();
    }

    fn load_projects(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(generation) =
            self.begin_read("projects", Step::Uses, Box::new(|c| c.load_projects()))
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        app.call(
            "list_projects",
            json!({"include_found":true}),
            move |_, result| {
                let Some(c) = weak
                    .upgrade()
                    .filter(|c| c.finish_read("projects", generation))
                else {
                    return;
                };
                let body = c.customize_box.clone();
                clear(&body);
                let copy = first_run_copy();
                let Some(projects) = result.ok().and_then(|v| {
                    serde_json::from_value::<Vec<Project>>(v["projects"].clone()).ok()
                }) else {
                    body.append(&body_label(copy.rules.unavailable));
                    let retry = gtk::Button::with_label(copy.rules.retry);
                    retry.connect_clicked({
                        let weak = Rc::downgrade(&c);
                        move |_| {
                            if let Some(c) = weak.upgrade() {
                                c.load_projects();
                            }
                        }
                    });
                    body.append(&retry);
                    return;
                };
                if projects.is_empty() {
                    body.append(&body_label(copy.rules.empty));
                }
                let choices = trace_commons_contributor::project_copy::contribution_mode_copy();
                for project in projects {
                    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    let label = gtk::Label::builder()
                        .label(&project.project_label)
                        .xalign(0.0)
                        .build();
                    label.add_css_class("tc-flow-emphasis");
                    label.set_tooltip_text(Some(&project.project_path));
                    row.append(&label);
                    let mut group: Option<gtk::CheckButton> = None;
                    // The daemon's mode shows until the person picks; only a
                    // pick is written (`RulesScreenLayout.rule`).
                    let shown = rules::shown_rule(&c.state.borrow(), &project);
                    let arms = body_label(copy.uses.automatic_after_sign_in);
                    arms.add_css_class("tc-meta");
                    arms.set_visible(rules::arms_after_sign_in(&c.state.borrow(), &project));
                    let project = Rc::new(project);
                    for rule in rules::offerable_modes(&project) {
                        let index = match rule {
                            Rule::Ask => 0,
                            Rule::Automatic => 1,
                            Rule::Never => 2,
                        };
                        let check = gtk::CheckButton::with_label(choices.choices[index].label);
                        check.set_group(group.as_ref());
                        if group.is_none() {
                            group = Some(check.clone());
                        }
                        check.set_active(shown == Some(rule));
                        check.connect_toggled({
                            let weak = Rc::downgrade(&c);
                            let project = project.clone();
                            let arms = arms.clone();
                            move |check| {
                                if check.is_active() {
                                    if let Some(c) = weak.upgrade() {
                                        rules::pick(&mut c.state.borrow_mut(), &project, rule);
                                        arms.set_visible(rules::arms_after_sign_in(
                                            &c.state.borrow(),
                                            &project,
                                        ));
                                        c.update_uses_action();
                                    }
                                }
                            }
                        });
                        row.append(&check);
                    }
                    // Automatic picked before sign-in is armed at Start, and
                    // says so in Uses' words (`armsAfterSignIn`).
                    row.append(&arms);
                    let history = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    let more = gtk::Expander::builder()
                        .label(copy.rules.past_sessions)
                        .child(&history)
                        .build();
                    more.connect_expanded_notify({
                        let weak = Rc::downgrade(&c);
                        let id = project.project_id.clone();
                        move |expander| {
                            if expander.is_expanded() {
                                if let Some(c) = weak.upgrade() {
                                    c.load_past(&id, &history);
                                }
                            }
                        }
                    });
                    row.append(&more);
                    body.append(&row);
                }
            },
        );
    }

    fn load_past(self: &Rc<Self>, id: &str, body: &gtk::Box) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let key = format!("past:{id}");
        let reload_id = id.to_owned();
        let reload_body = body.clone();
        let Some(generation) = self.begin_read(
            &key,
            Step::Uses,
            Box::new(move |c| c.load_past(&reload_id, &reload_body)),
        ) else {
            return;
        };
        let id = id.to_owned();
        let weak = Rc::downgrade(self);
        let body = body.clone();
        app.call(
            "list_past_sessions",
            json!({"project_id":id}),
            move |_, result| {
                let Some(c) = weak.upgrade().filter(|c| c.finish_read(&key, generation)) else {
                    return;
                };
                clear(&body);
                let rules = first_run_copy().rules;
                let Some(value) = result.ok() else {
                    body.append(&body_label(rules.sessions_unavailable));
                    return;
                };
                let Some(sessions) = value["sessions"].as_array() else {
                    body.append(&body_label(rules.sessions_unavailable));
                    return;
                };
                for session in sessions {
                    let Some(session_id) = session["session_id"].as_str() else {
                        continue;
                    };
                    let sid = session_id.to_owned();
                    let label = session["title"]
                        .as_str()
                        .or_else(|| session["started_at"].as_str())
                        .unwrap_or(rules.past_sessions);
                    let check = gtk::CheckButton::with_label(label);
                    check.set_sensitive(session["selectable"].as_bool() == Some(true));
                    check.set_active(
                        c.state
                            .borrow()
                            .past_selections
                            .get(&id)
                            .is_some_and(|v| v.contains(&sid)),
                    );
                    check.connect_toggled({
                        let weak = Rc::downgrade(&c);
                        let id = id.clone();
                        move |check| {
                            if let Some(c) = weak.upgrade() {
                                {
                                    let mut state = c.state.borrow_mut();
                                    let selected =
                                        state.past_selections.entry(id.clone()).or_default();
                                    selected.retain(|x| x != &sid);
                                    if check.is_active() {
                                        selected.push(sid.clone());
                                    }
                                }
                                c.update_uses_action();
                            }
                        }
                    });
                    body.append(&check);
                }
                if let Some(count) = value["not_listed"].as_u64().filter(|count| *count > 0) {
                    body.append(&body_label(&fill(
                        rules.not_listed,
                        "count",
                        &count.to_string(),
                    )));
                }
            },
        );
    }

    // ---- Join ----------------------------------------------------------

    fn load_join(self: &Rc<Self>) {
        if self.busy.get() {
            self.render_join();
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(generation) = self.begin_read("join", Step::Join, Box::new(|c| c.load_join()))
        else {
            return;
        };
        // Join reads the required use too: Start waits on it.
        if self.options.borrow().is_empty() {
            let weak = Rc::downgrade(self);
            app.call("consent_options", json!({}), move |_, result| {
                if let (Some(c), Some(options)) = (weak.upgrade(), parse_options(result)) {
                    *c.options.borrow_mut() = options;
                    let required = c.required();
                    uses::include_required(&mut c.state.borrow_mut(), required.as_deref());
                    c.render_join();
                }
            });
        }
        // An invite handed to this process waits for Join, after Terms.
        if let Some(invite) = super::onboarding::take_pending_invite() {
            let mut state = self.state.borrow_mut();
            if state.enrolled_invite.is_none() {
                state.invite = invite;
            }
        }
        let weak = Rc::downgrade(self);
        app.call("status", json!({}), move |_, result| {
            let Some(c) = weak.upgrade().filter(|c| c.finish_read("join", generation)) else {
                return;
            };
            let enrolled = result.ok().is_some_and(|v| {
                v["logged_in"].as_bool() == Some(true)
                    && v["tenant_id"].as_str().is_some_and(|id| !id.is_empty())
            });
            {
                let mut state = c.state.borrow_mut();
                state.enrollment_held = enrolled;
                // An earlier run's enrollment: held, with no invite to show.
                if enrolled && !state.signed_out_of_enrollment {
                    if state.enrolled_invite.is_none() && !state.near_ai_enrolled {
                        state.enrolled_invite = Some(String::new());
                    }
                    if state.account == Account::None {
                        state.account = Account::Enrolled;
                    }
                }
            }
            c.render_join();
            c.prepare_organization();
        });
    }

    fn render_join(self: &Rc<Self>) {
        let copy = first_run_copy();
        clear(&self.join_body);
        let state = self.state.borrow().clone();
        // As the Mac's Join when its account goes: the organization check
        // and its one automatic Start may run again.
        let forward = join::can_forward(&state);
        self.stage.join_signed_in(forward);
        if join::forward_lost(self.forward.replace(forward), forward) {
            self.org_checked.set(false);
            self.auto_started.set(false);
        }
        // near.ai, the one way in (owner, 2026-10-12).
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("tc-card");
        card.append(&eyebrow(copy.join.near_ai_eyebrow));
        card.append(&body_label(copy.join.near_ai_text));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if state.signed_in {
            let done = body_label(copy.join.signed_in);
            done.add_css_class("tc-clear");
            actions.append(&done);
        } else {
            if self.login_waiting.get() {
                let cancel = gtk::Button::with_label(copy.frame.cancel);
                cancel.connect_clicked({
                    let weak = Rc::downgrade(self);
                    move |_| {
                        if let Some(c) = weak.upgrade() {
                            c.cancel_login();
                        }
                    }
                });
                actions.append(&cancel);
            }
            let sign_in = gtk::Button::with_label(copy.join.near_ai_sign_in);
            sign_in.set_sensitive(!self.busy.get() && join::can_toggle_near_ai(&state));
            sign_in.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(c) = weak.upgrade() {
                        c.sign_in();
                    }
                }
            });
            actions.append(&sign_in);
        }
        card.append(&actions);
        self.join_body.append(&card);
        self.join_body.append(&self.referral_field());
        // Which organization NEAR AI credit goes to, once an enrollment is
        // held, as the Mac's Join draws `PayoutLinkView`.
        if state.holds_enrollment() {
            if let Some(app) = self.app.upgrade() {
                let view = self
                    .payout
                    .borrow_mut()
                    .get_or_insert_with(|| super::payout_link::PayoutLinkView::new(&app))
                    .clone();
                self.join_body.append(&view.root);
                if !self.payout_shown.replace(true) {
                    view.refresh();
                }
            }
        } else {
            self.payout_shown.set(false);
        }
        if join::shows_invite_card(&state) {
            self.join_body.append(&self.invite_card(&state));
        }
        let required_known = self.required().is_some();
        self.start.set_sensitive(join::can_start(
            &state,
            required_known,
            self.busy.get(),
            sharing_available(&state),
        ));
    }

    fn invite_card(self: &Rc<Self>, state: &State) -> gtk::Box {
        let copy = first_run_copy();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("tc-card");
        card.append(&eyebrow(copy.join.invite_eyebrow));
        let joined = state
            .enrolled_invite
            .as_deref()
            .is_some_and(|i| !i.trim().is_empty());
        let editable = state.enrolled_invite.is_none();
        if editable || !joined {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let entry = gtk::Entry::builder()
                .placeholder_text(copy.join.invite_placeholder)
                .hexpand(true)
                .sensitive(editable)
                .build();
            entry.set_text(
                self.invite_draft
                    .borrow()
                    .as_deref()
                    .unwrap_or(&state.invite),
            );
            entry.update_property(&[gtk::accessible::Property::Label(copy.join.invite_eyebrow)]);
            entry.connect_changed({
                let weak = Rc::downgrade(self);
                move |entry| {
                    if let Some(c) = weak.upgrade() {
                        *c.invite_draft.borrow_mut() = Some(entry.text().to_string());
                        c.invite_refused.set(false);
                    }
                }
            });
            let look_up = gtk::Button::with_label(copy.join.look_up);
            look_up.set_sensitive(editable);
            look_up.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(c) = weak.upgrade() {
                        c.look_up();
                    }
                }
            });
            row.append(&entry);
            row.append(&look_up);
            card.append(&row);
        }
        let host = trace_commons_contributor::commands::invite_issuer_host(state.invite.trim());
        let line = if joined {
            let host = host.unwrap_or_else(|| copy.join.unknown.to_owned());
            Some((
                fill(
                    &fill(copy.join.invite_joined, "host", &host),
                    "pay_range",
                    &join::pay_range(self.lookup.borrow().as_ref(), &copy.join),
                ),
                "tc-clear",
            ))
        } else if self.invite_refused.get() {
            Some((copy.join.invite_error.to_owned(), "tc-refused"))
        } else if self.invite_dead.get() {
            Some((copy.join.invite_dead.to_owned(), "tc-refused"))
        } else {
            host.map(|host| (host, "tc-meta"))
        };
        if let Some((text, class)) = line {
            let label = body_label(&text);
            label.add_css_class(class);
            card.append(&label);
        }
        card
    }

    /// The optional referral code under near.ai's Sign in (referral spec
    /// section 6), in the core's words; a code that is not one is said under
    /// the field, and never stops the sign-in.
    fn referral_field(self: &Rc<Self>) -> gtk::Box {
        let words = trace_commons_contributor::referral_copy::referral_copy();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.append(&eyebrow(words.sign_up_label));
        let entry = gtk::Entry::builder()
            .placeholder_text(words.field_placeholder)
            .hexpand(true)
            .sensitive(!self.busy.get())
            .build();
        entry.set_text(&self.referral.borrow());
        entry.update_property(&[gtk::accessible::Property::Label(words.sign_up_label)]);
        let problem = body_label("");
        problem.add_css_class("tc-refused");
        let show = {
            let problem = problem.clone();
            move |typed: &str| match join::referral_problem(typed) {
                Some(line) => {
                    problem.set_label(line);
                    problem.set_visible(true);
                }
                None => problem.set_visible(false),
            }
        };
        show(&self.referral.borrow());
        entry.connect_changed({
            let weak = Rc::downgrade(self);
            move |entry| {
                let text = entry.text().to_string();
                show(&text);
                if let Some(c) = weak.upgrade() {
                    *c.referral.borrow_mut() = text;
                }
            }
        });
        column.append(&entry);
        column.append(&problem);
        column
    }

    /// "Look up": the host is read here; the daemon looks the invite up and
    /// joins it when the person signs in. Not an invite: refused, not kept.
    /// An empty field withdraws the kept invite.
    fn look_up(self: &Rc<Self>) {
        let draft = self
            .invite_draft
            .borrow()
            .clone()
            .unwrap_or_else(|| self.state.borrow().invite.clone());
        let invite = draft.trim().to_owned();
        {
            let mut state = self.state.borrow_mut();
            if invite.is_empty() {
                state.invite.clear();
                self.invite_dead.set(false);
                *self.invite_draft.borrow_mut() = None;
            } else if trace_commons_contributor::commands::invite_issuer_host(&invite).is_some() {
                if invite != state.invite.trim() {
                    self.invite_dead.set(false);
                }
                state.invite = invite;
                self.invite_refused.set(false);
                *self.invite_draft.borrow_mut() = None;
            } else {
                self.invite_refused.set(true);
                if !self.invite_dead.get() {
                    state.invite.clear();
                }
            }
        }
        self.render_join();
    }

    /// near.ai chosen and signed in at once: the daemon is running.
    fn sign_in(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        self.error.set_visible(false);
        join::choose_near_ai(&mut self.state.borrow_mut());
        self.sync_tools();
        self.commit(
            Commit::SignIn,
            Box::new(|c, failure| {
                if let Some(line) = failure {
                    c.fail(&line);
                }
                c.render_join();
                c.prepare_organization();
            }),
        );
        self.render_join();
    }

    /// Single-organization setup once a NEAR AI account is signed in: with
    /// exactly one cloud organization it is selected for spending and linked
    /// for payouts, and Start runs by itself (`SingleOrganizationOnboarding`).
    /// Every other answer leaves Start to the person.
    fn prepare_organization(self: &Rc<Self>) {
        let state = self.state.borrow().clone();
        if self.org_checked.get()
            || self.busy.get()
            || state.step != Step::Join
            || !join::can_forward(&state)
            || state.completed
        {
            return;
        }
        self.org_checked.set(true);
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let generation = self.generation.get();
        self.set_busy(true);
        let weak = Rc::downgrade(self);
        let finish = Rc::new(move |outcome: org::Outcome| {
            if let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) {
                c.set_busy(false);
                c.render_join();
                // The setup may have linked the organization: read the credit
                // section's rail again.
                let payout = c.payout.borrow().clone();
                if let Some(view) = payout {
                    view.refresh();
                }
                let ready = c.state.borrow().clone();
                if outcome == org::Outcome::Ready
                    && !c.auto_started.replace(true)
                    && join::can_start(
                        &ready,
                        c.required().is_some(),
                        false,
                        sharing_available(&ready),
                    )
                {
                    c.start();
                }
            }
        });
        app.call("near_ai_spend_organizations", json!({"fresh":true}), {
            move |app, result| {
                let id = match result.as_ref().map(org::after_cloud) {
                    Ok(Ok(id)) => id,
                    Ok(Err(outcome)) => return finish(outcome),
                    Err(_) => return finish(org::Outcome::Unavailable),
                };
                app.call("payout_link_orgs", json!({}), move |app, result| {
                    match result.as_ref().map(|owned| org::after_owned(owned, &id)) {
                        Ok(Ok(_)) => {}
                        Ok(Err(outcome)) => return finish(outcome),
                        Err(_) => return finish(org::Outcome::Unavailable),
                    }
                    app.call("payout_rails_status", json!({}), move |app, result| {
                        let rail = match result.as_ref().map(|s| org::after_rails(s, &id)) {
                            Ok(Ok(rail)) => rail,
                            Ok(Err(outcome)) => return finish(outcome),
                            Err(_) => return finish(org::Outcome::Unavailable),
                        };
                        select_then_link(app, id, rail, finish);
                    });
                });
            }
        });
    }

    /// End the browser wait. The daemon is asked to end its attempt, and
    /// the wait ends here whatever it answers (fail closed): the step and
    /// the near.ai choice stay, nothing is said.
    fn cancel_login(self: &Rc<Self>) {
        if !self.login_waiting.replace(false) {
            return;
        }
        let attempt = self.login_attempt.borrow_mut().take();
        if let Some(app) = self.app.upgrade() {
            let params = match &attempt {
                Some(id) => json!({"attempt_id": id}),
                None => json!({}),
            };
            app.call("near_ai_credential_cancel", params, |_, _| {});
        }
        let ticket = self.runner.borrow().current().cloned();
        if let Some(ticket) = ticket.filter(|t| t.action == Action::NearAiLogin) {
            self.resolved(ticket, Outcome::Cancelled);
        }
    }

    /// The near.ai browser login (`FirstRunRunner`'s `nearAILogin`): signed
    /// in already, or open the browser and poll until it is, the person
    /// ends it, or the limit passes.
    fn near_ai_login(self: &Rc<Self>, ticket: Ticket) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let weak = Rc::downgrade(self);
        app.call(
            "near_ai_credential_status",
            json!({}),
            move |app, result| {
                let Some(c) = weak.upgrade() else {
                    return;
                };
                if result
                    .as_ref()
                    .is_ok_and(|v| join::login_verdict(v) == join::LoginPoll::SignedIn)
                {
                    return c.resolved(ticket, Outcome::Success);
                }
                let weak = Rc::downgrade(&c);
                app.call("near_ai_credential_start", json!({}), move |app, result| {
                    let Some(c) = weak.upgrade() else {
                        return;
                    };
                    let attempt = result
                        .as_ref()
                        .ok()
                        .and_then(|v| v["attempt_id"].as_str())
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned);
                    let opened = result
                        .as_ref()
                        .ok()
                        .and_then(|v| v["browser_url"].as_str())
                        .filter(|url| !url.is_empty())
                        .is_some_and(super::credential::open_browser);
                    if !opened {
                        if let Some(id) = attempt {
                            app.call(
                                "near_ai_credential_cancel",
                                json!({"attempt_id": id}),
                                |_, _| {},
                            );
                        }
                        return c.resolved(
                            ticket,
                            Outcome::Failure(first_run_copy().folders.sign_in_failed.into()),
                        );
                    }
                    *c.login_attempt.borrow_mut() = attempt;
                    c.login_waiting.set(true);
                    c.set_busy(true);
                    c.render_join();
                    c.poll_login(ticket);
                });
            },
        );
    }

    fn poll_login(self: &Rc<Self>, ticket: Ticket) {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(join::LOGIN_LIMIT_SECS);
        let weak = Rc::downgrade(self);
        let polling = Rc::new(Cell::new(false));
        glib::timeout_add_seconds_local(1, move || {
            let Some(c) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !c.login_waiting.get() || c.runner.borrow().current() != Some(&ticket) {
                return glib::ControlFlow::Break;
            }
            if std::time::Instant::now() >= deadline {
                c.login_waiting.set(false);
                c.resolved(
                    ticket.clone(),
                    Outcome::Failure(first_run_copy().folders.sign_in_failed.into()),
                );
                return glib::ControlFlow::Break;
            }
            if polling.replace(true) {
                return glib::ControlFlow::Continue;
            }
            let Some(app) = c.app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let params = match c.login_attempt.borrow().as_ref() {
                Some(id) => json!({"attempt_id": id}),
                None => json!({}),
            };
            let weak = Rc::downgrade(&c);
            let polling = polling.clone();
            let ticket = ticket.clone();
            app.call("near_ai_credential_status", params, move |_, result| {
                polling.set(false);
                let Some(c) = weak.upgrade() else {
                    return;
                };
                if !c.login_waiting.get() || c.runner.borrow().current() != Some(&ticket) {
                    return;
                }
                let verdict = result
                    .as_ref()
                    .map_or(join::LoginPoll::Waiting, join::login_verdict);
                let outcome = match verdict {
                    join::LoginPoll::Waiting => return,
                    join::LoginPoll::SignedIn => Outcome::Success,
                    join::LoginPoll::Cancelled => Outcome::Cancelled,
                    join::LoginPoll::Ended => {
                        Outcome::Failure(first_run_copy().folders.sign_in_failed.into())
                    }
                };
                c.login_waiting.set(false);
                c.login_attempt.borrow_mut().take();
                c.resolved(ticket, outcome);
            });
            glib::ControlFlow::Continue
        });
    }

    // ---- Start sharing -------------------------------------------------

    fn start(self: &Rc<Self>) {
        if self.closed.get() || self.busy.get() {
            return;
        }
        self.error.set_visible(false);
        if self.receipt().is_none() {
            self.fail(first_run_copy().uses.complete_failed);
            return;
        }
        {
            // What Start sends matches what Uses showed: the required use
            // included, and a held near.ai enrollment as the account.
            let required = self.required();
            let mut state = self.state.borrow_mut();
            join::ready_to_start(&mut state);
            uses::include_required(&mut state, required.as_deref());
        }
        let automatic = {
            let state = self.state.borrow();
            state.sharing == Sharing::Automatic && state.holds_enrollment()
        };
        self.set_busy(true);
        if automatic {
            self.disclose_scrub();
        } else {
            self.commit_start();
        }
    }
    fn disclose_scrub(self: &Rc<Self>) {
        // Before a grant there is no certificate evidence of model scrubbing.
        // The core's configuration-only selector deliberately says patterns.
        let grant = consent_copy::automatic_contribution_copy(None);
        let body = page(first_run_copy().uses.sharing);
        for line in [
            grant.summary,
            grant.no_review,
            grant.scope_required,
            grant.raw_send,
        ] {
            body.append(&body_label(line));
        }
        if let Some(scrub) = grant.patterns_only.or(grant.model_scrubbed) {
            body.append(&body_label(scrub.scope));
            body.append(&body_label(scrub.limit));
        }
        self.disclosure(body, {
            let weak = Rc::downgrade(self);
            move || {
                if let Some(c) = weak.upgrade() {
                    c.disclose_witness();
                }
            }
        });
    }
    fn disclosure(self: &Rc<Self>, body: gtk::Box, on_continue: impl Fn() + 'static) {
        let Some(parent) = self.window.upgrade() else {
            return;
        };
        let dialog = adw::Window::builder()
            .transient_for(&parent)
            .destroy_with_parent(true)
            .modal(true)
            .default_width(580)
            .default_height(600)
            .build();
        StageScheme::hold(&dialog);
        let next = gtk::Button::with_label(first_run_copy().frame.continue_button);
        body.append(&next);
        let acknowledged = Rc::new(Cell::new(false));
        next.connect_clicked({
            let weak = Rc::downgrade(self);
            let generation = self.generation.get();
            let dialog = dialog.downgrade();
            let acknowledged = acknowledged.clone();
            move |_| {
                if !weak.upgrade().is_some_and(|c| c.valid(generation))
                    || acknowledged.replace(true)
                {
                    return;
                }
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
                on_continue();
            }
        });
        dialog.connect_close_request({
            let weak = Rc::downgrade(self);
            move |_| {
                if !acknowledged.get() {
                    if let Some(c) = weak.upgrade() {
                        c.state.borrow_mut().grant_ready = false;
                        c.set_busy(false);
                        c.render_join();
                    }
                }
                glib::Propagation::Proceed
            }
        });
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("tc-root");
        content.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&body)
                .build(),
        );
        dialog.set_content(Some(&content));
        dialog.present();
    }
    fn disclose_witness(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        app.call("route_disclosure", json!({}), move |_, result| {
            let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                return;
            };
            let Some(facts) = result
                .ok()
                .filter(|v| consent_copy::route_disclosure_for_wire(v).is_some())
            else {
                c.fail(first_run_copy().uses.sharing_unavailable);
                return;
            };
            let panel = crate::disclosure::panel(Some(&facts));
            let body = page(&panel.title);
            super::fill_disclosure_rows(&body, &panel.rows);
            let witness = facts["witness"]["signing_address"]
                .as_str()
                .map(str::to_owned);
            c.disclosure(body, {
                let weak = Rc::downgrade(&c);
                move || {
                    if let Some(c) = weak.upgrade() {
                        c.authorize_grant(witness.clone());
                    }
                }
            });
        });
    }
    fn authorize_grant(self: &Rc<Self>, witness: Option<String>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        app.call("status", json!({}), move |_, result| {
            let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                return;
            };
            let connected = result
                .ok()
                .is_some_and(|v| v["logged_in"].as_bool() == Some(true));
            let progress = flow1::Flow1Progress {
                connected,
                scopes_saved: Some(c.state.borrow().scopes.clone()),
                path: Some(flow1::ContributionPath::Automatic),
                scrub_disclosure_seen: true,
                witness_disclosure_seen: true,
                witness_shown: witness,
            };
            match flow1::grant_request(&progress) {
                Ok(witness) => {
                    let mut state = c.state.borrow_mut();
                    state.grant_ready = true;
                    state.witness = witness.map(str::to_owned);
                }
                Err(_) => {
                    c.fail(first_run_copy().uses.sharing_unavailable);
                    return;
                }
            }
            c.commit_start();
        });
    }
    fn commit_start(self: &Rc<Self>) {
        self.commit(
            Commit::Start,
            Box::new(|c, failure| {
                if let Some(line) = failure {
                    c.fail(&line);
                    c.render_join();
                    return;
                }
                if !c.state.borrow().completed {
                    c.fail(first_run_copy().uses.complete_failed);
                    c.render_join();
                    return;
                }
                if c.runner.borrow().refused_grant.is_some() {
                    c.fail(first_run_copy().uses.sharing_refused);
                    c.start.set_visible(false);
                } else if let Some(window) = c.window.upgrade() {
                    window.close();
                }
            }),
        );
    }

    // ---- Commits -------------------------------------------------------

    /// Run `commit`'s calls in order, then `done` with the failure line, if
    /// any. The accepted terms are handed over once, before the first
    /// sign-in or Start (`FirstRunRunner.handOverTerms`); a refusal is
    /// logged and does not stop the commit.
    fn commit(self: &Rc<Self>, commit: Commit, done: Done) {
        if self.closed.get() {
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if self.runner.borrow().busy() {
            // Never silently: whoever asked is told, and can ask again.
            let line = if commit == Commit::Roots {
                first_run_copy().folders.settings_failed
            } else {
                first_run_copy().uses.complete_failed
            };
            return done(self, Some(line.into()));
        }
        self.set_busy(true);
        *self.done.borrow_mut() = Some(done);
        let receipt = self.receipt();
        let hand_over = receipt.is_some()
            && commit != Commit::Roots
            && self.state.borrow_mut().take_terms_handoff();
        let weak = Rc::downgrade(self);
        let run = move || {
            let Some(c) = weak.upgrade() else {
                return;
            };
            let ticket = c.runner.borrow_mut().begin(&c.state.borrow(), commit);
            match ticket {
                Some(ticket) => c.execute(ticket),
                None if commit == Commit::Start => {
                    c.finish_commit(Some(first_run_copy().uses.complete_failed.into()))
                }
                None => c.finish_commit(None),
            }
        };
        match receipt.filter(|_| hand_over) {
            Some(receipt) => app.call("terms_accept", receipt.params(), move |_, result| {
                if crate::first_run::terms_handoff_refused(&result) {
                    eprintln!("trace-commons-shell: terms-handoff-refused");
                }
                run();
            }),
            None => run(),
        }
    }

    fn finish_commit(self: &Rc<Self>, failure: Option<String>) {
        self.set_busy(false);
        let done = self.done.borrow_mut().take();
        if let Some(done) = done {
            done(self, failure);
        }
    }

    fn execute(self: &Rc<Self>, ticket: Ticket) {
        if self.closed.get() {
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if ticket.action == Action::NearAiLogin {
            return self.near_ai_login(ticket);
        }
        if ticket.action == Action::MarkComplete {
            let weak = Rc::downgrade(self);
            let generation = self.generation.get();
            app.call("get_setup_status", json!({}), move |app, result| {
                let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                    return;
                };
                let Some(scope) = result
                    .ok()
                    .filter(|v| v["can_complete"].as_bool() == Some(true))
                    .and_then(|v| v.get("scope").cloned())
                    .filter(|v| !v.is_null())
                else {
                    c.resolved(
                        ticket,
                        Outcome::Failure(first_run_copy().uses.complete_failed.into()),
                    );
                    return;
                };
                let weak = Rc::downgrade(&c);
                let refused = c.runner.borrow().refused_grant.is_some();
                app.call(
                    "complete_setup",
                    json!({"expected_scope":scope}),
                    move |app, result| {
                        let ok = result
                            .ok()
                            .is_some_and(|v| v["complete"].as_bool() == Some(true));
                        if ok && refused {
                            app.toast(first_run_copy().uses.sharing_refused);
                        }
                        let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                            return;
                        };
                        c.resolved(
                            ticket,
                            if ok {
                                Outcome::Success
                            } else {
                                Outcome::Failure(first_run_copy().uses.complete_failed.into())
                            },
                        );
                    },
                );
            });
            return;
        }
        let Some((method, mut params)) = ticket.action.ipc() else {
            let line = failure_copy(&ticket.action, "");
            self.resolved(ticket, Outcome::Failure(line.into()));
            return;
        };
        if ticket.action == Action::EnrollNearAi {
            if let Some(code) = join::sent_referral_code(&self.referral.borrow()) {
                params["referral_code"] = json!(code);
            }
        }
        if matches!(ticket.action, Action::LookupInvite(_)) {
            self.lookup.borrow_mut().take();
        }
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        app.call(method, params, move |_, result| {
            let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                return;
            };
            let outcome = match result {
                Ok(value) if crate::first_run::acknowledged(&ticket.action, &value) => {
                    if matches!(ticket.action, Action::LookupInvite(_)) {
                        *c.lookup.borrow_mut() = Some(value);
                    }
                    Outcome::Success
                }
                Ok(_) if matches!(ticket.action, Action::GrantAutomatic { .. }) => {
                    Outcome::GrantRefused("automatic-grant-unconfirmed".into())
                }
                Err(label) if matches!(ticket.action, Action::GrantAutomatic { .. }) => {
                    Outcome::GrantRefused(label)
                }
                Ok(value) => {
                    // An invite the daemon answered as not valid is dead; its
                    // own label says why.
                    let label = value["reason_label"].as_str().unwrap_or(
                        if ticket.action == Action::EnrollNearAi {
                            trace_commons_contributor::private_inference_copy::NEAR_AI_ENROLL_UNAVAILABLE
                        } else {
                            "invite-invalid"
                        },
                    );
                    c.note_lookup(&ticket.action, label);
                    Outcome::Failure(failure_copy(&ticket.action, label).into())
                }
                Err(label) => {
                    c.note_lookup(&ticket.action, &label);
                    Outcome::Failure(failure_copy(&ticket.action, &label).into())
                }
            };
            c.resolved(ticket, outcome);
        });
    }

    /// A refused invite is said on its card; a lookup that never reached an
    /// answer says nothing about the invite.
    fn note_lookup(&self, action: &Action, label: &str) {
        if matches!(action, Action::LookupInvite(_)) {
            self.invite_dead.set(!lookup_unavailable(label));
        }
    }

    fn resolved(self: &Rc<Self>, ticket: Ticket, outcome: Outcome) {
        let next = self
            .runner
            .borrow_mut()
            .resolve(&ticket, &mut self.state.borrow_mut(), outcome);
        if let Some(ticket) = next {
            self.execute(ticket);
            return;
        }
        if self.runner.borrow().busy() {
            return;
        }
        let failure = self.runner.borrow().failure.clone();
        self.finish_commit(failure);
    }
}

/// The core's words for the sharing path chosen are all there.
fn sharing_available(state: &State) -> bool {
    uses::sharing_available(
        state.sharing,
        &consent_copy::automatic_contribution_copy(None),
    )
}

fn parse_options(result: Result<Value, String>) -> Option<Vec<ScopeOption>> {
    result
        .ok()?
        .get("scopes")?
        .as_array()?
        .iter()
        .map(ScopeOption::from_wire)
        .collect::<Option<Vec<_>>>()
        .filter(|o| !o.is_empty())
}

/// Select the one organization for spending, read it back, then link it
/// for payouts unless it already is, and read the rail back
/// (`SingleOrganizationOnboarding.prepare`).
fn select_then_link(app: &Rc<App>, id: String, rail: org::Rail, finish: Rc<dyn Fn(org::Outcome)>) {
    app.call(
        "near_ai_spend_organization_select",
        json!({"organization_id": id}),
        move |app, result| {
            if result.is_err() {
                return finish(org::Outcome::Unavailable);
            }
            app.call(
                "near_ai_spend_organizations",
                json!({"fresh":true}),
                move |app, result| {
                    match result.as_ref().map(|s| org::after_selected(s, &id)) {
                        Ok(Ok(())) => {}
                        Ok(Err(outcome)) => return finish(outcome),
                        Err(_) => return finish(org::Outcome::Unavailable),
                    }
                    link_then_confirm(app, id, rail, finish);
                },
            );
        },
    );
}

fn link_then_confirm(app: &Rc<App>, id: String, rail: org::Rail, finish: Rc<dyn Fn(org::Outcome)>) {
    let confirm = {
        let id = id.clone();
        let finish = finish.clone();
        move |app: &Rc<App>| {
            app.call("payout_rails_status", json!({}), move |_, result| {
                finish(match result {
                    Ok(status) => org::confirmed(&status, &id),
                    Err(_) => org::Outcome::Unavailable,
                });
            });
        }
    };
    let org::Rail::Link { version } = rail else {
        return confirm(app);
    };
    // Selecting inference can refresh the credential used to prove cloud
    // membership: read the payout revision again before the link.
    app.call("payout_link_orgs", json!({}), move |app, result| {
        let revision = match result.as_ref().map(|owned| org::after_owned(owned, &id)) {
            Ok(Ok(revision)) => revision,
            Ok(Err(outcome)) => return finish(outcome),
            Err(_) => return finish(org::Outcome::Unavailable),
        };
        app.call(
            "payout_link_start",
            json!({"rail_id": org::NEAR_AI_RAIL, "organization_id": id,
                "orgs_revision": revision, "expected_current_version": version}),
            move |app, result| {
                if result.is_err() {
                    return finish(org::Outcome::Unavailable);
                }
                app.call(
                    "payout_link_confirm",
                    json!({"rail_id": org::NEAR_AI_RAIL, "action": "link",
                        "organization_id": id, "confirmed": true}),
                    move |app, result| {
                        if result.is_err() {
                            return finish(org::Outcome::Unavailable);
                        }
                        confirm(app);
                    },
                );
            },
        );
    });
}

/// The labels that say a lookup never reached an answer.
fn lookup_unavailable(label: &str) -> bool {
    label.is_empty()
        || matches!(
            label,
            "invite-lookup-unavailable" | "unparseable-response" | "missing-result"
        )
}

fn failure_copy(action: &Action, label: &str) -> &'static str {
    let copy = first_run_copy();
    match action {
        Action::SetSourceSettings(_) => copy.folders.settings_failed,
        Action::LookupInvite(_) if lookup_unavailable(label) => copy.folders.lookup_unavailable,
        Action::LookupInvite(_) => copy.join.invite_dead,
        Action::Enroll(_) => copy.folders.enroll_refused,
        Action::SignInNearAi | Action::NearAiLogin => copy.folders.sign_in_failed,
        Action::EnrollNearAi => {
            trace_commons_contributor::private_inference_copy::near_ai_enroll_line(label)
        }
        Action::SetAskEachTime => copy.uses.sharing_save_failed,
        Action::SetConsentScopes(_) => copy.uses.scopes_failed,
        Action::SetProjectMode { .. } | Action::IncludePastSessions { .. } => {
            copy.uses.rules_failed
        }
        Action::SetPrivateAi(_) => copy.uses.private_ai_failed,
        _ => copy.uses.complete_failed,
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;

    #[test]
    fn each_sign_in_failure_has_the_core_line_the_mac_shows() {
        let copy = first_run_copy();
        let lookup = Action::LookupInvite("i".into());
        assert_eq!(
            failure_copy(&lookup, "invite-lookup-unavailable"),
            copy.folders.lookup_unavailable
        );
        assert_eq!(failure_copy(&lookup, "expired"), copy.join.invite_dead);
        assert_eq!(
            failure_copy(&Action::Enroll("i".into()), "enroll-failed"),
            copy.folders.enroll_refused
        );
        assert_eq!(
            failure_copy(&Action::NearAiLogin, ""),
            copy.folders.sign_in_failed
        );
        assert_eq!(
            failure_copy(&Action::SetSourceSettings(Default::default()), ""),
            copy.folders.settings_failed
        );
    }

    #[test]
    fn a_consent_list_with_a_malformed_row_is_no_list() {
        let good = json!({"scopes":[{"name":"a","title":"A","always_on":true}]});
        assert_eq!(parse_options(Ok(good)).map(|o| o.len()), Some(1));
        let bad = json!({"scopes":[{"name":"a","always_on":true}]});
        assert!(parse_options(Ok(bad)).is_none());
        assert!(parse_options(Ok(json!({"scopes":[]}))).is_none());
        assert!(parse_options(Err("x".into())).is_none());
    }
}

#[cfg(test)]
mod dark_tests {
    use super::super::stage::{STAGE_CLASS, StageMode, current_mode};
    use super::*;

    fn luminance(c: &gtk::gdk::RGBA) -> f64 {
        let ch = |v: f32| {
            let v = f64::from(v);
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.red()) + 0.7152 * ch(c.green()) + 0.0722 * ch(c.blue())
    }

    fn contrast(a: &gtk::gdk::RGBA, b: &gtk::gdk::RGBA) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn settle(context: &gtk::glib::MainContext) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(300);
        while std::time::Instant::now() < end {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// The ground a window is painted on: a probe label coloured with
    /// libadwaita's `--window-bg-color` (scoped mode) or the palette's
    /// `tc_bg` (app-wide mode), read where it sits.
    fn ground(root: &gtk::Box) -> gtk::gdk::RGBA {
        if current_mode() == StageMode::AppWide {
            return root.style_context().lookup_color("tc_bg").expect("tc_bg");
        }
        let probe = gtk::Label::new(Some("probe"));
        probe.set_widget_name("tc-test-ground-probe");
        root.append(&probe);
        let provider = gtk::CssProvider::new();
        provider.load_from_data("#tc-test-ground-probe { color: var(--window-bg-color); }");
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
        let end = std::time::Instant::now() + std::time::Duration::from_millis(100);
        while std::time::Instant::now() < end {
            gtk::glib::MainContext::default().iteration(false);
        }
        probe.style_context().color()
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn welcome_is_readable_in_dark_and_title_is_larger_than_body() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        // The person's desktop is light; the stage is dark on its own.
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
        super::super::style::install();
        super::super::community_brand::install();
        let copy = first_run_copy();
        let window = adw::Window::builder().default_width(680).build();
        StageScheme::hold(&window);
        match current_mode() {
            StageMode::AppWide => assert!(adw::StyleManager::default().is_dark()),
            StageMode::Scoped => {
                assert!(window.has_css_class(STAGE_CLASS));
                assert!(
                    !adw::StyleManager::default().is_dark(),
                    "the scheme stays the person's"
                );
            }
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
        let welcome = page(copy.welcome.title);
        let body = body_label(copy.welcome.body);
        welcome.append(&body);
        root.append(&welcome);
        window.set_content(Some(&root));
        window.present();
        settle(&context);
        let title = welcome.first_child().unwrap();
        let ground = ground(&root);
        let body_colour = body.style_context().color();
        let title_colour = title.style_context().color();
        assert!(
            luminance(&ground) < 0.2,
            "the stage ground {ground} is dark"
        );
        assert!(
            contrast(&body_colour, &ground) >= 4.5,
            "body {body_colour} on {ground}"
        );
        assert!(
            contrast(&title_colour, &ground) >= 4.5,
            "title {title_colour} on {ground}"
        );
        let size = |w: &gtk::Widget| {
            let layout = w
                .clone()
                .downcast::<gtk::Label>()
                .unwrap()
                .create_pango_layout(Some("Ag"));
            layout.pixel_size().1
        };
        assert!(
            size(&title) > size(body.upcast_ref()),
            "title not larger than body"
        );
        window.destroy();
        assert!(
            !adw::StyleManager::default().is_dark(),
            "the person's scheme is put back"
        );
    }

    /// GTK 4.16 and libadwaita 1.6 or later: the stage is dark and the main
    /// window beside it keeps the person's light scheme. Below that the
    /// stage is app-wide and this has nothing to check: it says so and
    /// passes, so the weston job (GTK 4.14) runs it as a no-op.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_scoped_stage_leaves_the_main_window_light() {
        assert_eq!(std::env::consts::OS, "linux");
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        if current_mode() != StageMode::Scoped {
            eprintln!(
                "not run: GTK {}.{} / libadwaita {}.{} uses the app-wide stage",
                gtk::major_version(),
                gtk::minor_version(),
                adw::major_version(),
                adw::minor_version()
            );
            return;
        }
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
        super::super::style::install();
        let body_in = |window: &adw::Window| {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
            root.add_css_class("tc-root");
            let label = body_label("Ag");
            root.append(&label);
            window.set_content(Some(&root));
            window.present();
            (root, label)
        };
        let main = adw::Window::builder().default_width(400).build();
        let (main_root, main_label) = body_in(&main);
        let stage = adw::Window::builder().default_width(400).build();
        StageScheme::hold(&stage);
        let (stage_root, stage_label) = body_in(&stage);
        settle(&context);
        assert!(!adw::StyleManager::default().is_dark());
        let (main_ground, stage_ground) = (ground(&main_root), ground(&stage_root));
        assert!(
            luminance(&main_ground) > 0.5,
            "main ground {main_ground} is light"
        );
        assert!(
            luminance(&stage_ground) < 0.2,
            "stage ground {stage_ground} is dark"
        );
        assert!(contrast(&main_label.style_context().color(), &main_ground) >= 4.5);
        assert!(contrast(&stage_label.style_context().color(), &stage_ground) >= 4.5);
        assert_ne!(
            main_label.style_context().color(),
            stage_label.style_context().color()
        );
        stage.destroy();
        main.destroy();
    }
}
