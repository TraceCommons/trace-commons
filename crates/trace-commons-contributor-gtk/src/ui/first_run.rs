//! GTK first run: answers remain local until the ordered Start commit.
use super::{
    App,
    first_run_terms::{Accepted, TermsPage},
    onboarding::body_label,
};
use crate::{
    first_run::{Account, Action, Commit, Outcome, Rule, Runner, Sharing, State, Step, Ticket},
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

#[derive(Default)]
struct ReadRequests {
    active: std::collections::BTreeMap<String, u64>,
    pending: std::collections::BTreeMap<String, (u64, DeferredRead)>,
}

struct Coordinator {
    app: Weak<App>,
    window: glib::WeakRef<adw::Window>,
    stack: gtk::Stack,
    heading: adw::WindowTitle,
    state: RefCell<State>,
    runner: RefCell<Runner>,
    accepted: RefCell<Option<Rc<RefCell<Option<Accepted>>>>>,
    error: gtk::Label,
    back: gtk::Button,
    start: gtk::Button,
    sharing_ask: RefCell<Option<gtk::CheckButton>>,
    busy: Cell<bool>,
    closed: Cell<bool>,
    generation: Cell<u64>,
    reads: RefCell<ReadRequests>,
    join: gtk::Box,
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
fn page(title: &str) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.set_margin_start(32);
    root.set_margin_end(32);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    let label = body_label(title);
    label.add_css_class("tc-brand-dialog-title");
    root.append(&label);
    root
}
fn title(a: &str, b: &str) -> String {
    [a, b].join(" ")
}
fn continue_button(c: &Rc<Coordinator>, body: &gtk::Box) -> gtk::Button {
    let button = gtk::Button::with_label(first_run_copy().frame.continue_button);
    button.add_css_class("suggested-action");
    button.connect_clicked({
        let weak = Rc::downgrade(c);
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.next();
            }
        }
    });
    body.append(&button);
    button
}
fn clear(root: &gtk::Box) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
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
    let stack = gtk::Stack::builder()
        .vexpand(true)
        .vhomogeneous(false)
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .build();
    let heading = adw::WindowTitle::new(crate::copy::APP_NAME, "");
    let error = body_label("");
    error.add_css_class("tc-refused");
    error.set_visible(false);
    let back = gtk::Button::with_label(copy.frame.back);
    let start = gtk::Button::with_label(copy.uses.start);
    start.add_css_class("suggested-action");
    start.set_sensitive(false);
    let c = Rc::new(Coordinator {
        app: Rc::downgrade(app),
        window: window.downgrade(),
        stack: stack.clone(),
        heading: heading.clone(),
        state: RefCell::new(State {
            daemon_started: true,
            ..State::default()
        }),
        runner: RefCell::new(Runner::default()),
        accepted: RefCell::new(None),
        error: error.clone(),
        back: back.clone(),
        start: start.clone(),
        sharing_ask: RefCell::new(None),
        busy: Cell::new(false),
        closed: Cell::new(false),
        generation: Cell::new(0),
        reads: RefCell::new(ReadRequests::default()),
        join: gtk::Box::new(gtk::Orientation::Vertical, 16),
    });
    let welcome = page(copy.welcome.title);
    welcome.append(&body_label(copy.welcome.body));
    continue_button(&c, &welcome);
    stack.add_named(&welcome, Some("welcome"));
    let tools = super::roots::connected(app, {
        let weak = Rc::downgrade(&c);
        move || {
            if let Some(c) = weak.upgrade() {
                c.next();
            }
        }
    });
    stack.add_named(&tools, Some("tools"));
    let private_ai = page(&title(
        copy.private_ai.title_light,
        copy.private_ai.title_bold,
    ));
    private_ai.append(&body_label(copy.private_ai.offer_lead));
    let detail = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let private_copy = trace_commons_contributor::private_inference_copy::private_inference_copy();
    for text in [
        copy.private_ai.offer_more,
        private_copy.offer_exposure,
        private_copy.offer_no_repoint,
    ] {
        detail.append(&body_label(text));
    }
    let more = gtk::Expander::builder()
        .label(copy.private_ai.learn_more)
        .child(&detail)
        .build();
    private_ai.append(&more);
    continue_button(&c, &private_ai);
    stack.add_named(&private_ai, Some("private-ai"));
    let traces = page(&title(copy.traces.title_light, copy.traces.title_bold));
    let folders = gtk::Box::new(gtk::Orientation::Vertical, 12);
    traces.append(&folders);
    let next = continue_button(&c, &traces);
    next.set_sensitive(false);
    let retry = gtk::Button::with_label(copy.rules.retry);
    traces.append(&retry);
    retry.connect_clicked({
        let weak = Rc::downgrade(&c);
        let folders = folders.clone();
        let next = next.clone();
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.load_projects(&folders, &next);
            }
        }
    });
    traces.connect_map({
        let weak = Rc::downgrade(&c);
        let folders = folders.clone();
        let next = next.clone();
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.load_projects(&folders, &next);
            }
        }
    });
    stack.add_named(&traces, Some("traces"));
    let uses = page(&title(copy.uses.title_light, copy.uses.title_bold));
    let scopes = gtk::Box::new(gtk::Orientation::Vertical, 12);
    uses.append(&scopes);
    let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
    let ask = gtk::CheckButton::with_label(modes.choices[0].label);
    ask.set_active(true);
    let automatic = gtk::CheckButton::with_label(modes.choices[1].label);
    automatic.set_group(Some(&ask));
    *c.sharing_ask.borrow_mut() = Some(ask.clone());
    uses.append(&body_label(copy.uses.sharing));
    uses.append(&ask);
    uses.append(&body_label(modes.choices[0].line));
    uses.append(&automatic);
    uses.append(&body_label(modes.choices[1].line));
    uses.append(&body_label(copy.uses.automatic_after_sign_in));
    automatic.connect_toggled({
        let weak = Rc::downgrade(&c);
        move |button| {
            if let Some(c) = weak.upgrade() {
                let mut state = c.state.borrow_mut();
                state.sharing = if button.is_active() {
                    Sharing::Automatic
                } else {
                    Sharing::Ask
                };
                state.grant_ready = false;
            }
        }
    });
    let next = continue_button(&c, &uses);
    next.set_sensitive(false);
    let retry = gtk::Button::with_label(copy.rules.retry);
    uses.append(&retry);
    retry.connect_clicked({
        let weak = Rc::downgrade(&c);
        let scopes = scopes.clone();
        let next = next.clone();
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.load_scopes(&scopes, &next);
            }
        }
    });
    uses.connect_map({
        let weak = Rc::downgrade(&c);
        let scopes = scopes.clone();
        let next = next.clone();
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.load_scopes(&scopes, &next);
            }
        }
    });
    stack.add_named(&uses, Some("uses"));
    let terms = TermsPage::new({
        let weak = Rc::downgrade(&c);
        move || {
            if let Some(c) = weak.upgrade() {
                c.next();
            }
        }
    });
    *c.accepted.borrow_mut() = Some(terms.accepted);
    stack.add_named(&terms.root, Some("terms"));
    let join = page(&title(copy.join.title_light, copy.join.title_bold));
    join.append(&body_label(copy.join.body));
    join.append(&c.join);
    join.append(&start);
    stack.add_named(&join, Some("join"));
    join.connect_map({
        let weak = Rc::downgrade(&c);
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.load_join();
            }
        }
    });
    start.connect_clicked({
        let weak = Rc::downgrade(&c);
        move |_| {
            if let Some(c) = weak.upgrade() {
                c.start();
            }
        }
    });
    back.connect_clicked({
        let weak = Rc::downgrade(&c);
        move |_| {
            if let Some(c) = weak.upgrade() {
                if !c.busy.get() {
                    c.state.borrow_mut().back();
                    c.show();
                }
            }
        }
    });
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    content.append(&adw::HeaderBar::builder().title_widget(&heading).build());
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&stack)
        .build();
    content.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    footer.set_margin_start(24);
    footer.set_margin_end(24);
    footer.set_margin_top(12);
    footer.set_margin_bottom(16);
    error.set_hexpand(true);
    back.set_halign(gtk::Align::End);
    footer.append(&error);
    footer.append(&back);
    content.append(&footer);
    window.set_content(Some(&content));
    window.connect_close_request({
        let c = c.clone();
        move |_| {
            c.closed.set(true);
            c.generation.set(c.generation.get().wrapping_add(1));
            c.runner.borrow_mut().cancel();
            glib::Propagation::Proceed
        }
    });
    if super::roots::take_first_run_handoff() {
        c.state.borrow_mut().step = Step::PrivateAi;
    }
    c.show();
    window.present();
    window
}
impl Coordinator {
    fn show(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        let step = self.state.borrow().step;
        let copy = first_run_copy();
        self.heading.set_subtitle(match step {
            Step::Welcome => copy.welcome.title,
            Step::Tools => copy.frame.step_tools,
            Step::PrivateAi => copy.frame.step_private_ai,
            Step::Traces => copy.frame.step_traces,
            Step::Uses => copy.frame.step_uses,
            Step::Terms => copy.frame.step_terms,
            Step::Join => copy.frame.step_join,
        });
        self.stack.set_visible_child_name(name(step));
        self.back.set_visible(step != Step::Welcome);
        self.error.set_visible(false);
    }
    fn next(&self) {
        if !self.busy.get() {
            self.state.borrow_mut().next();
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
        self.stack.set_sensitive(!busy);
        self.back.set_sensitive(!busy);
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
    fn load_scopes(self: &Rc<Self>, body: &gtk::Box, next: &gtk::Button) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let reload_body = body.clone();
        let reload_next = next.clone();
        let Some(generation) = self.begin_read(
            "scopes",
            Step::Uses,
            Box::new(move |c| c.load_scopes(&reload_body, &reload_next)),
        ) else {
            return;
        };
        next.set_sensitive(false);
        let weak = Rc::downgrade(self);
        let body = body.clone();
        let next = next.clone();
        app.call("consent_options", json!({}), move |_, result| {
            let Some(c) = weak
                .upgrade()
                .filter(|c| c.finish_read("scopes", generation))
            else {
                return;
            };
            let Some(options) = result
                .ok()
                .and_then(|v| v.get("scopes").and_then(Value::as_array).cloned())
            else {
                c.fail(first_run_copy().uses.scopes_failed);
                return;
            };
            if options.is_empty()
                || options.iter().any(|v| {
                    v["name"].as_str().is_none()
                        || v["title"].as_str().is_none()
                        || v["always_on"].as_bool().is_none()
                })
            {
                c.fail(first_run_copy().uses.scopes_failed);
                return;
            }
            clear(&body);
            for option in options {
                let key = option["name"].as_str().unwrap().to_owned();
                let required = option["always_on"].as_bool() == Some(true);
                if required && !c.state.borrow().scopes.contains(&key) {
                    c.state.borrow_mut().scopes.push(key.clone());
                }
                let check = gtk::CheckButton::with_label(option["title"].as_str().unwrap());
                check.set_active(c.state.borrow().scopes.contains(&key));
                check.set_sensitive(!required);
                body.append(&check);
                if let Some(description) = option["description"].as_str() {
                    body.append(&body_label(description));
                }
                check.connect_toggled({
                    let weak = Rc::downgrade(&c);
                    move |check| {
                        if let Some(c) = weak.upgrade() {
                            let mut state = c.state.borrow_mut();
                            state.scopes.retain(|s| s != &key);
                            if check.is_active() {
                                state.scopes.push(key.clone());
                            }
                            state.grant_ready = false;
                        }
                    }
                });
            }
            next.set_sensitive(true);
        });
    }
    fn load_projects(self: &Rc<Self>, body: &gtk::Box, next: &gtk::Button) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let reload_body = body.clone();
        let reload_next = next.clone();
        let Some(generation) = self.begin_read(
            "projects",
            Step::Traces,
            Box::new(move |c| c.load_projects(&reload_body, &reload_next)),
        ) else {
            return;
        };
        next.set_sensitive(false);
        let weak = Rc::downgrade(self);
        let body = body.clone();
        let next = next.clone();
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
                let Some((projects, count)) = result.ok().and_then(|v| {
                    serde_json::from_value::<Vec<Project>>(v["projects"].clone())
                        .ok()
                        .map(|projects| (projects, trace_count(&v)))
                }) else {
                    c.fail(first_run_copy().rules.unavailable);
                    return;
                };
                clear(&body);
                if let Some(count) = count {
                    let copy = first_run_copy();
                    let label = if count == 1 {
                        copy.frame.session_count_one.to_owned()
                    } else {
                        copy.frame
                            .session_count
                            .replace("{count}", &count.to_string())
                    };
                    body.append(&body_label(&label));
                    if count > 0 {
                        body.append(&body_label(copy.traces.found_could_credit));
                    }
                }
                if projects.is_empty() {
                    body.append(&body_label(first_run_copy().rules.empty));
                }
                for project in projects {
                    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    row.append(&body_label(&project.project_label));
                    row.append(&body_label(&project.project_path));
                    let choices = trace_commons_contributor::project_copy::contribution_mode_copy();
                    if let Some(current) = choices
                        .choices
                        .iter()
                        .find(|choice| choice.mode == project.mode)
                    {
                        row.append(&body_label(current.label));
                    }
                    let mut group: Option<gtk::CheckButton> = None;
                    for (index, rule) in [Rule::Ask, Rule::Automatic, Rule::Never]
                        .into_iter()
                        .enumerate()
                    {
                        if rule == Rule::Automatic && project.is_unresolved_bucket {
                            continue;
                        }
                        let check = gtk::CheckButton::with_label(choices.choices[index].label);
                        check.set_group(group.as_ref());
                        if group.is_none() {
                            group = Some(check.clone());
                        }
                        // Only an explicit override selects a radio button. The daemon
                        // mode above reports the current policy without promising its future value.
                        check.set_active(
                            c.state.borrow().rules.get(&project.project_id).copied() == Some(rule),
                        );
                        let id = project.project_id.clone();
                        check.connect_toggled({
                            let weak = Rc::downgrade(&c);
                            move |check| {
                                if check.is_active() {
                                    if let Some(c) = weak.upgrade() {
                                        c.state.borrow_mut().rules.insert(id.clone(), rule);
                                    }
                                }
                            }
                        });
                        row.append(&check);
                    }
                    let history = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    let more = gtk::Expander::builder()
                        .label(first_run_copy().rules.past_sessions)
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
                next.set_sensitive(true);
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
            Step::Traces,
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
                let Some(value) = result.ok() else {
                    body.append(&body_label(first_run_copy().rules.sessions_unavailable));
                    return;
                };
                let Some(sessions) = value["sessions"].as_array() else {
                    body.append(&body_label(first_run_copy().rules.sessions_unavailable));
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
                        .unwrap_or(first_run_copy().rules.past_sessions);
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
                                let mut state = c.state.borrow_mut();
                                let selected = state.past_selections.entry(id.clone()).or_default();
                                selected.retain(|x| x != &sid);
                                if check.is_active() {
                                    selected.push(sid.clone());
                                }
                            }
                        }
                    });
                    body.append(&check);
                }
                if let Some(count) = value["not_listed"].as_u64().filter(|count| *count > 0) {
                    body.append(&body_label(
                        &first_run_copy()
                            .rules
                            .not_listed
                            .replace("{count}", &count.to_string()),
                    ));
                }
            },
        );
    }
    fn load_join(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(generation) = self.begin_read("join", Step::Join, Box::new(|c| c.load_join()))
        else {
            return;
        };
        self.start.set_sensitive(false);
        self.join.set_sensitive(false);
        let weak = Rc::downgrade(self);
        app.call("status", json!({}), move |app, result| {
            let Some(c) = weak.upgrade().filter(|c| c.finish_read("join", generation)) else {
                return;
            };
            let enrolled = result.ok().is_some_and(|v| {
                v["logged_in"].as_bool() == Some(true)
                    && v["tenant_id"].as_str().is_some_and(|id| !id.is_empty())
            });
            c.state.borrow_mut().enrollment_held = enrolled;
            c.state.borrow_mut().account = if enrolled {
                Account::Enrolled
            } else {
                Account::None
            };
            clear(&c.join);
            c.join.set_sensitive(true);
            if enrolled {
                c.join.append(&body_label(first_run_copy().join.signed_in));
                c.start.set_sensitive(true);
                return;
            }
            // Acceptance is handed off before any sign-in mutation. A failed handoff
            // leaves the daemon's terms hold intact, matching the native client.
            let Some(receipt) = c.receipt() else {
                c.state.borrow_mut().step = Step::Terms;
                c.show();
                return;
            };
            c.set_busy(true);
            let weak = Rc::downgrade(&c);
            app.call("terms_accept", receipt.params(), move |app, _result| {
                let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                    return;
                };
                c.set_busy(false);
                let Some(window) = c.window.upgrade() else {
                    return;
                };
                let surface = super::onboarding::join_surface(app, &window, {
                    let weak = Rc::downgrade(&c);
                    move || {
                        if let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) {
                            c.load_join();
                        }
                    }
                });
                c.join.append(&surface);
            });
        });
    }
    fn start(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        if self.busy.replace(true) {
            return;
        }
        self.set_busy(true);
        self.error.set_visible(false);
        if self.receipt().is_none() {
            self.fail(first_run_copy().uses.complete_failed);
            return;
        }
        if self.state.borrow().sharing == Sharing::Automatic {
            self.disclose_scrub();
        } else {
            self.commit_terms();
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
                acknowledged.set(true);
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
                    }
                }
                glib::Propagation::Proceed
            }
        });
        dialog.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&body)
                .build(),
        ));
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
            c.commit_terms();
        });
    }
    fn commit_terms(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(receipt) = self.receipt() else {
            self.fail(first_run_copy().uses.complete_failed);
            return;
        };
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        app.call("terms_accept", receipt.params(), move |_, _| {
            let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                return;
            };
            let ticket = c
                .runner
                .borrow_mut()
                .begin(&c.state.borrow(), Commit::Start);
            if let Some(ticket) = ticket {
                c.execute(ticket)
            } else {
                c.fail(first_run_copy().uses.complete_failed)
            }
        });
    }
    fn execute(self: &Rc<Self>, ticket: Ticket) {
        if self.closed.get() {
            return;
        }
        let Some(app) = self.app.upgrade() else {
            return;
        };
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
        let Some((method, params)) = ticket.action.ipc() else {
            self.resolved(
                ticket,
                Outcome::Failure(first_run_copy().uses.complete_failed.into()),
            );
            return;
        };
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        app.call(method, params, move |_, result| {
            let Some(c) = weak.upgrade().filter(|c| c.valid(generation)) else {
                return;
            };
            let outcome = match result {
                Ok(value) if crate::first_run::acknowledged(&ticket.action, &value) => {
                    Outcome::Success
                }
                Ok(_) if matches!(ticket.action, Action::GrantAutomatic { .. }) => {
                    Outcome::GrantRefused("automatic-grant-unconfirmed".into())
                }
                Ok(_) => Outcome::Failure(failure_copy(&ticket.action).into()),
                Err(label) if matches!(ticket.action, Action::GrantAutomatic { .. }) => {
                    Outcome::GrantRefused(label)
                }
                Err(_) => Outcome::Failure(failure_copy(&ticket.action).into()),
            };
            c.resolved(ticket, outcome);
        });
    }
    fn resolved(self: &Rc<Self>, ticket: Ticket, outcome: Outcome) {
        let next = self
            .runner
            .borrow_mut()
            .resolve(&ticket, &mut self.state.borrow_mut(), outcome);
        if self.state.borrow().sharing == Sharing::Ask {
            if let Some(ask) = self.sharing_ask.borrow().as_ref() {
                ask.set_active(true);
            }
        }
        if let Some(ticket) = next {
            self.execute(ticket);
            return;
        }
        if let Some(failure) = self.runner.borrow().failure.clone() {
            self.fail(&failure);
            return;
        }
        if self.state.borrow().completed {
            if self.runner.borrow().refused_grant.is_some() {
                self.fail(first_run_copy().uses.sharing_refused);
                self.start.set_visible(false);
            } else if let Some(window) = self.window.upgrade() {
                window.close();
            }
        } else {
            self.set_busy(false);
        }
    }
}
fn failure_copy(action: &Action) -> &'static str {
    let copy = first_run_copy();
    match action {
        Action::SetAskEachTime => copy.uses.sharing_save_failed,
        Action::SetConsentScopes(_) => copy.uses.scopes_failed,
        Action::SetProjectMode { .. } | Action::IncludePastSessions { .. } => {
            copy.uses.rules_failed
        }
        Action::SetPrivateAi(_) => copy.uses.private_ai_failed,
        _ => copy.uses.complete_failed,
    }
}

/// Missing or malformed per-folder counts are unknown, never zero.
fn trace_count(value: &Value) -> Option<u64> {
    value
        .get("projects")?
        .as_array()?
        .iter()
        .try_fold(0u64, |total, row| {
            total.checked_add(row.get("session_count")?.as_u64()?)
        })
}

#[cfg(test)]
mod read_tests {
    use super::*;
    #[test]
    fn trace_summary_requires_all_counts_and_checked_sum() {
        assert_eq!(
            trace_count(&json!({"projects":[{"session_count":2},{"session_count":3}]})),
            Some(5)
        );
        assert_eq!(trace_count(&json!({"projects":[]})), Some(0));
        for value in [
            json!({}),
            json!({"projects":[{}]}),
            json!({"projects":[{"session_count":-1}]}),
            json!({"projects":[{"session_count":u64::MAX},{"session_count":1}]}),
        ] {
            assert_eq!(trace_count(&value), None);
        }
    }
}
