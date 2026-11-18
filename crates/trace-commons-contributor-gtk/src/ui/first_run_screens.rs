//! The first run's quieter screens, drawn as the Mac draws them: Welcome
//! with Get started and the agent prompt (`WelcomeScreen`,
//! `AgentConfigurationScreen`), the Private AI diagram and its count
//! (`PrivateAIScreen`), and the two diagrams Traces and Uses stand under.
//! Every word is the core's; the diagrams are drawn, not written.
use adw::prelude::*;
use trace_commons_contributor::first_run_copy::first_run_copy;

use super::onboarding::body_label;
use crate::first_run::tools::Found;

pub(super) fn page(title: &str) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.set_margin_start(32);
    root.set_margin_end(32);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    // Its own label, not `body_label` plus a title class: two type classes on
    // one label leave the size to whichever rule is declared later.
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-flow-title");
    root.append(&label);
    root
}

pub(super) fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(&format!("{{{key}}}"), value)
}

/// Welcome: the title, the line under it, Get started, and the agent
/// prompt's action below it. No Back: nothing comes before it.
pub(super) fn welcome(on_start: impl Fn() + 'static, on_agent: impl Fn() + 'static) -> gtk::Box {
    let copy = first_run_copy();
    let root = page(copy.welcome.title);
    root.append(&body_label(copy.welcome.body));
    let start = gtk::Button::with_label(copy.welcome.start);
    start.add_css_class("suggested-action");
    start.add_css_class("pill");
    start.set_halign(gtk::Align::Start);
    start.connect_clicked(move |_| on_start());
    root.append(&start);
    let agent = gtk::Button::with_label(copy.agent_configuration.action);
    agent.set_halign(gtk::Align::Start);
    agent.connect_clicked(move |_| on_agent());
    root.append(&agent);
    root
}

/// The agent prompt, guidance only: opening or copying it changes nothing.
/// Close returns to Welcome; Back is the frame's alone.
pub(super) fn agent_configuration(on_close: impl Fn() + 'static) -> gtk::Box {
    let words = first_run_copy().agent_configuration;
    let root = page(words.title);
    root.append(&body_label(words.body));
    let prerequisite = body_label(words.prerequisite);
    prerequisite.add_css_class("tc-meta");
    root.append(&prerequisite);
    let prompt = gtk::Label::builder()
        .label(words.prompt)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    prompt.add_css_class("tc-flow-body");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(200)
        .vexpand(true)
        .child(&prompt)
        .build();
    scroll.add_css_class("card");
    root.append(&scroll);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let close = gtk::Button::with_label(words.close);
    close.connect_clicked(move |_| on_close());
    let copy_button = gtk::Button::with_label(words.copy);
    copy_button.add_css_class("suggested-action");
    copy_button.set_hexpand(true);
    copy_button.set_halign(gtk::Align::End);
    copy_button.connect_clicked(move |button| {
        button.clipboard().set_text(words.prompt);
        button.set_label(words.copied);
    });
    row.append(&close);
    row.append(&copy_button);
    root.append(&row);
    root
}

/// Private AI: the tools that can connect, each one's route into the hub,
/// the hub's line on to NEAR AI, then "N of M tools can connect" and the
/// offer. The step has no switch; Settings turns Private AI on.
pub(super) fn private_ai_body(connectable: &[Found], found: usize) -> gtk::Box {
    let copy = first_run_copy();
    let private = trace_commons_contributor::private_inference_copy::private_inference_copy();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let names: Vec<&'static str> = connectable
        .iter()
        .map(|f| super::first_run_tools::tool_name(&f.source))
        .collect();
    root.append(&route_diagram(&names, private.stat_inference_access));
    let figure = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let number = gtk::Label::new(Some(&connectable.len().to_string()));
    number.add_css_class("tc-flow-title");
    figure.append(&number);
    let template = if found == 1 {
        copy.private_ai.can_connect_one
    } else {
        copy.private_ai.can_connect
    };
    figure.append(&body_label(&fill(template, "total", &found.to_string())));
    root.append(&figure);
    root.append(&body_label(copy.private_ai.offer_lead));
    let detail = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for text in [
        copy.private_ai.offer_more,
        private.offer_exposure,
        private.offer_no_repoint,
    ] {
        detail.append(&body_label(text));
    }
    root.append(
        &gtk::Expander::builder()
            .label(copy.private_ai.learn_more)
            .child(&detail)
            .build(),
    );
    root
}

fn ink(widget: &gtk::Widget) -> gtk::gdk::RGBA {
    #[allow(deprecated)]
    widget.style_context().color()
}

fn dash(cr: &gtk::cairo::Context, colour: &gtk::gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        f64::from(colour.red()),
        f64::from(colour.green()),
        f64::from(colour.blue()),
        alpha,
    );
    cr.set_line_width(2.0);
    cr.set_dash(&[5.0, 6.0], 0.0);
}

/// The Private AI routes: one dashed curve from each named tool into the
/// hub, and one on from the hub to the destination's pill.
fn route_diagram(names: &[&'static str], destination: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::Center);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_valign(gtk::Align::Center);
    for name in names {
        let label = gtk::Label::new(Some(name));
        label.add_css_class("tc-flow-emphasis");
        label.set_xalign(1.0);
        column.append(&label);
    }
    row.append(&column);
    let area = gtk::DrawingArea::builder()
        .content_width(200)
        .content_height(names.len().max(1) as i32 * 34)
        .build();
    let count = names.len();
    area.set_draw_func(move |area, cr, width, height| {
        let colour = ink(area.upcast_ref());
        let (w, h) = (f64::from(width), f64::from(height));
        let hub = (w * 0.6, h / 2.0);
        dash(cr, &colour, 0.6);
        for index in 0..count {
            let y = if count > 1 {
                h * (index as f64 + 0.5) / count as f64
            } else {
                h / 2.0
            };
            cr.move_to(0.0, y);
            cr.curve_to(w * 0.3, y, w * 0.35, hub.1, hub.0 - 14.0, hub.1);
            let _ = cr.stroke();
        }
        cr.move_to(hub.0 + 14.0, hub.1);
        cr.line_to(w, hub.1);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        cr.arc(hub.0, hub.1, 12.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    row.append(&area);
    let pill = gtk::Label::new(Some(destination));
    pill.add_css_class("tc-chip");
    pill.set_valign(gtk::Align::Center);
    row.append(&pill);
    row
}

/// The first run's one column: every step at the same width, at most the
/// Mac frame's 560 points (`FirstRunFrame`), centred in a wider window.
pub(super) fn column(child: &impl IsA<gtk::Widget>) -> adw::Clamp {
    adw::Clamp::builder()
        .maximum_size(560)
        .tightening_threshold(560)
        .child(child)
        .build()
}

/// The Uses flow: this machine, a dashed line through the review gate to
/// the commons. The gate reads the core's "You review" on Ask and the mode
/// table's Automatic on Automatic. The Mac's caption under this machine is
/// "This Mac", which no shared word replaces yet, so it has none here.
pub(super) fn uses_flow(gate: &str) -> gtk::Box {
    let copy = first_run_copy();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.set_halign(gtk::Align::Center);
    let area = gtk::DrawingArea::builder()
        .content_width(360)
        .content_height(56)
        .build();
    area.set_draw_func(|area, cr, width, height| {
        let colour = ink(area.upcast_ref());
        let (w, h) = (f64::from(width), f64::from(height));
        let y = h / 2.0;
        let xs = [w * 0.15, w * 0.5, w * 0.85];
        dash(cr, &colour, 0.5);
        cr.move_to(xs[0] + 16.0, y);
        cr.line_to(xs[2] - 16.0, y);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        for (index, x) in xs.iter().enumerate() {
            cr.arc(
                *x,
                y,
                if index == 1 { 18.0 } else { 14.0 },
                0.0,
                std::f64::consts::TAU,
            );
            let _ = cr.stroke();
        }
    });
    root.append(&area);
    let captions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    captions.set_homogeneous(true);
    // The Mac's "This Mac", for any computer (owner, 2026-11-17).
    for text in [copy.uses.this_computer, gate, copy.uses.the_commons] {
        let label = gtk::Label::new(Some(text));
        label.add_css_class("tc-meta");
        captions.append(&label);
    }
    root.append(&captions);
    root
}

/// Traces: the watched tools' traces counted, and the lines under the
/// count -- the daemon's (`first_run_copy`) when it answered, else the
/// core's line and, while the use that earns credit is ticked, its note.
pub(super) fn traces_lines(
    found: Option<u64>,
    served: Option<&serde_json::Value>,
    earns_credit: bool,
) -> Option<(String, Option<String>)> {
    let found = found.filter(|n| *n > 0)?;
    let copy = first_run_copy();
    if let Some(served) = served {
        let credit = served["found_credit"].as_str();
        let could = served["found_could_credit"].as_str();
        if let (Some(credit), Some(could)) = (credit, could) {
            return Some(if earns_credit {
                (
                    credit.to_owned(),
                    served["credit_depends"].as_str().map(str::to_owned),
                )
            } else {
                (could.to_owned(), None)
            });
        }
    }
    let line = if earns_credit {
        trace_commons_contributor::first_run_copy::traces_estimate_line(&copy.traces, Some(found))
    } else {
        trace_commons_contributor::first_run_copy::traces_could_earn_line(&copy.traces, Some(found))
    }?;
    Some((
        line,
        earns_credit.then(|| copy.traces.credit_depends.to_owned()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn traces_say_nothing_until_something_is_found() {
        assert_eq!(traces_lines(None, None, true), None);
        assert_eq!(traces_lines(Some(0), None, true), None);
    }

    #[test]
    fn traces_read_the_daemon_first_and_drop_the_note_without_credit() {
        let copy = first_run_copy();
        let served = json!({"found_credit":"A","found_could_credit":"B","credit_depends":"C"});
        assert_eq!(
            traces_lines(Some(3), Some(&served), true),
            Some(("A".into(), Some("C".into())))
        );
        assert_eq!(
            traces_lines(Some(3), Some(&served), false),
            Some(("B".into(), None))
        );
        assert_eq!(
            traces_lines(Some(3), None, true),
            Some((
                copy.traces.found_credit.into(),
                Some(copy.traces.credit_depends.into())
            ))
        );
        assert_eq!(
            traces_lines(Some(3), None, false),
            Some((copy.traces.found_could_credit.into(), None))
        );
    }
}

/// Mounted checks of each screen, run under a Linux display:
/// `--ignored --test-threads=1`.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod mounted_tests {
    use super::*;
    use crate::first_run::tools::{Found, Tools, Watches};
    use std::{cell::RefCell, rc::Rc};

    /// The daemon's answer as Tools reads it: `held` watched, with the
    /// declaration revisions `revisions`.
    fn watching(held: &[&str], revisions: &[(&str, u64)]) -> Watches {
        Watches {
            held: held.iter().map(|s| (*s).to_owned()).collect(),
            revisions: revisions
                .iter()
                .map(|(tool, revision)| ((*tool).to_owned(), *revision))
                .collect(),
        }
    }

    fn init() -> glib::MainContext {
        let context = glib::MainContext::default();
        adw::init().expect("GTK display unavailable");
        context
    }

    fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut result = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(widget) = pending.pop() {
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
            result.push(widget);
        }
        result
    }

    fn buttons(root: &gtk::Box) -> Vec<String> {
        descendants(root.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .filter_map(|b| b.label().map(|l| l.to_string()))
            .collect()
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn welcome_offers_get_started_and_the_agent_prompt() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let copy = first_run_copy();
        let root = welcome(|| {}, || {});
        let labels = buttons(&root);
        assert!(labels.iter().any(|l| l == copy.welcome.start));
        assert!(labels.iter().any(|l| l == copy.agent_configuration.action));
        assert!(!labels.iter().any(|l| l == copy.frame.continue_button));
        let agent = agent_configuration(|| {});
        let labels = buttons(&agent);
        assert!(labels.iter().any(|l| l == copy.agent_configuration.copy));
        assert!(labels.iter().any(|l| l == copy.agent_configuration.close));
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn tools_start_unticked_and_continue_waits_for_a_watch() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let mut tools = Tools::default();
        tools.record_discovery(vec![
            Found {
                source: "claude-code".into(),
                path: "/home/x/.claude/projects".into(),
                exists: true,
                session_count: 1,
                total_bytes: None,
            },
            Found {
                source: "codex".into(),
                path: "/home/x/.codex/sessions".into(),
                exists: false,
                session_count: 0,
                total_bytes: None,
            },
        ]);
        let tools = Rc::new(RefCell::new(tools));
        let window = gtk::Window::new();
        let screen = super::super::first_run_tools::build(tools.clone(), &window, || {});
        let checks: Vec<_> = descendants(screen.root.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .collect();
        assert_eq!(checks.len(), 1, "a tool not found is not asked");
        assert!(!checks[0].is_active());
        assert!(!screen.continue_button.is_sensitive());
        checks[0].set_active(true);
        assert!(screen.continue_button.is_sensitive());
        assert!(tools.borrow().watches_anything());
    }

    /// After the old-portal command the daemon watches the folder it
    /// declared: Tools shows that tool ticked, with no path of discovery's,
    /// and Continue works (#2171 review, finding 3).
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn tools_tick_a_tool_the_daemon_already_watches() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let mut tools = Tools::default();
        // Discovery reports every known store, found or not.
        tools.record_discovery(vec![
            Found {
                source: "claude-code".into(),
                path: "/home/x/.claude/projects".into(),
                exists: true,
                session_count: 1,
                total_bytes: None,
            },
            Found {
                source: "codex".into(),
                path: "/home/x/.codex/sessions".into(),
                exists: false,
                session_count: 0,
                total_bytes: None,
            },
        ]);
        let tools = Rc::new(RefCell::new(tools));
        let window = gtk::Window::new();
        let screen = super::super::first_run_tools::build(tools.clone(), &window, || {});
        let checks = || -> Vec<gtk::CheckButton> {
            descendants(screen.root.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
                .collect()
        };
        assert!(!checks()[0].is_active());
        assert!(!screen.continue_button.is_sensitive());
        screen.record_watches(watching(&["claude-code"], &[]));
        let check = checks()[0].clone();
        assert!(check.is_active());
        assert_eq!(check.tooltip_text(), None);
        assert!(screen.continue_button.is_sensitive());
        let roots = tools.borrow().session_roots().unwrap();
        assert!(!roots.contains_key("claude_source"), "{roots:?}");
    }

    /// A folder the old Documents portal (v4) cannot map: the failure line,
    /// then the owner's approved line, the fixed command, and "Copy
    /// command", which puts exactly that command on the clipboard. A later
    /// commit that succeeds hides them again.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn a_folder_the_old_portal_cannot_map_shows_the_line_and_a_copy_only_command() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let window = gtk::Window::new();
        let screen = super::super::first_run_tools::build(
            Rc::new(RefCell::new(Tools::default())),
            &window,
            || {},
        );
        let failed = first_run_copy().folders.settings_failed;
        let hint = super::super::roots::host_folder_hint("claude_source").expect("approved");
        screen.finish(Some(failed));
        screen.show_host_hint(Some(&hint));
        let root: gtk::Widget = screen.root.clone().upcast();
        let shown = |widget: &gtk::Widget| {
            let mut current = Some(widget.clone());
            while let Some(w) = current {
                if !w.is_visible() {
                    return false;
                }
                if w == root {
                    return true;
                }
                current = w.parent();
            }
            false
        };
        let labels: Vec<String> = descendants(&root)
            .into_iter()
            .filter(|w| shown(w))
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .map(|l| l.text().to_string())
            .collect();
        let command =
            trace_commons_contributor::daemon::service::host_folder_command("claude_source")
                .unwrap();
        for text in [
            failed,
            "To add this folder anyway, open a terminal in it and run:",
            command.as_str(),
        ] {
            assert!(labels.iter().any(|l| l == text), "{text} not shown");
        }
        // Wrapped, the command shows no hyphen at the break, as the
        // service dialogs' commands do.
        let command_label = descendants(&root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .find(|l| l.text() == command.as_str())
            .expect("the command's label");
        assert!(super::super::service_dialog::draws_no_hyphen(
            &command_label
        ));
        let copy = descendants(&root)
            .into_iter()
            .filter(|w| shown(w))
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| {
                b.label().as_deref()
                    == Some(
                        trace_commons_contributor::managed::copy::background_service::COPY_COMMAND,
                    )
            })
            .expect("Copy command");
        copy.emit_clicked();
        let text = Rc::new(RefCell::new(None));
        let read = Rc::clone(&text);
        copy.clipboard()
            .read_text_async(None::<&gtk::gio::Cancellable>, move |r| {
                *read.borrow_mut() = Some(r.ok().flatten().map(|s| s.to_string()));
            });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while text.borrow().is_none() && std::time::Instant::now() < deadline {
            context.iteration(true);
        }
        assert_eq!(text.borrow().clone().flatten(), Some(command.clone()));
        screen.finish(None);
        assert!(
            !descendants(&root)
                .into_iter()
                .filter(|w| shown(w))
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .any(|l| l.text() == command),
            "a commit that succeeded still shows the command"
        );
    }

    /// The old-portal command run, the daemon newly holds the refused tool:
    /// Tools stops saying the change couldn't be saved and stops showing
    /// the command. Another tool newly held leaves both shown (#2171
    /// review 2, finding 1).
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn a_refusal_hides_once_the_daemon_holds_the_refused_tool() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let mut tools = Tools::default();
        tools.record_discovery(vec![
            Found {
                source: "claude-code".into(),
                path: "/home/x/.claude/projects".into(),
                exists: true,
                session_count: 1,
                total_bytes: None,
            },
            Found {
                source: "codex".into(),
                path: "/home/x/.codex/sessions".into(),
                exists: true,
                session_count: 1,
                total_bytes: None,
            },
        ]);
        let window = gtk::Window::new();
        let screen =
            super::super::first_run_tools::build(Rc::new(RefCell::new(tools)), &window, || {});
        let failed = first_run_copy().folders.settings_failed;
        let hint = super::super::roots::host_folder_hint("claude_source").expect("approved");
        // The coordinator's order: the hint, then the failure line.
        screen.show_host_hint(Some(&hint));
        screen.finish(Some(failed));
        let root: gtk::Widget = screen.root.clone().upcast();
        let shown = |text: &str| {
            descendants(&root)
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .filter(|l| l.text() == text)
                .any(|l| {
                    let mut current: Option<gtk::Widget> = Some(l.upcast());
                    while let Some(w) = current {
                        if !w.is_visible() {
                            return false;
                        }
                        if w == root {
                            return true;
                        }
                        current = w.parent();
                    }
                    false
                })
        };
        assert!(shown(failed));
        assert!(shown(&hint.command));
        screen.record_watches(watching(&["codex"], &[]));
        assert!(shown(failed), "another tool held hid the failure line");
        assert!(shown(&hint.command), "another tool held hid the command");
        screen.record_watches(watching(&["codex", "claude-code"], &[]));
        assert!(
            !shown(failed),
            "the refused tool is held: failure line still shown"
        );
        assert!(
            !shown(&hint.command),
            "the refused tool is held: command still shown"
        );
        assert!(screen.continue_button.is_sensitive());
    }

    /// A refusal for another folder of a tool the daemon already watches:
    /// the tool stays held, so only its declaration's revision says the
    /// command landed. Another tool's revision moving leaves the failure
    /// line and the command shown; the refused tool's hides both (#2171
    /// review 2, finding 4).
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn a_refusal_for_a_held_tool_hides_once_its_declaration_is_written() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let window = gtk::Window::new();
        let screen = super::super::first_run_tools::build(
            Rc::new(RefCell::new(Tools::default())),
            &window,
            || {},
        );
        screen.record_watches(watching(&["claude-code", "codex"], &[("codex", 2)]));
        let failed = first_run_copy().folders.settings_failed;
        let hint = super::super::roots::host_folder_hint("claude_source").expect("approved");
        screen.show_host_hint(Some(&hint));
        screen.finish(Some(failed));
        let root: gtk::Widget = screen.root.clone().upcast();
        let shown = |label: &gtk::Widget| {
            let mut current = Some(label.clone());
            while let Some(w) = current {
                if !w.is_visible() {
                    return false;
                }
                if w == root {
                    return true;
                }
                current = w.parent();
            }
            false
        };
        let failure: gtk::Widget = screen.failure.clone().upcast();
        assert!(shown(&failure));
        screen.record_watches(watching(&["claude-code", "codex"], &[("codex", 2)]));
        assert!(shown(&failure), "an unchanged answer hid the failure line");
        screen.record_watches(watching(&["claude-code", "codex"], &[("codex", 3)]));
        assert!(shown(&failure), "another tool's write hid the failure line");
        screen.record_watches(watching(
            &["claude-code", "codex"],
            &[("claude-code", 1), ("codex", 3)],
        ));
        assert!(
            !shown(&failure),
            "the refused tool's declaration landed: failure line still shown"
        );
        let command = descendants(&root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .find(|l| l.text() == hint.command.as_str())
            .map(|l| l.upcast::<gtk::Widget>());
        assert!(
            command.is_none_or(|l| !shown(&l)),
            "the refused tool's declaration landed: command still shown"
        );
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn private_ai_counts_the_tools_that_can_connect() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let found = Found {
            source: "codex".into(),
            path: "/x".into(),
            exists: true,
            session_count: 0,
            total_bytes: None,
        };
        let body = private_ai_body(&[found], 2);
        let words = fill(first_run_copy().private_ai.can_connect, "total", "2");
        assert!(
            descendants(body.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .any(|l| l.text() == words)
        );
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_uses_flow_names_its_gate_and_the_commons() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let copy = first_run_copy();
        let flow = uses_flow(copy.uses.you_review);
        let labels: Vec<String> = descendants(flow.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .map(|l| l.text().to_string())
            .collect();
        assert!(labels.iter().any(|l| l == copy.uses.you_review));
        assert!(labels.iter().any(|l| l == copy.uses.the_commons));
        assert!(labels.iter().any(|l| l == copy.uses.this_computer));
        assert!(!labels.iter().any(|l| l.contains("Mac")));
    }
}
