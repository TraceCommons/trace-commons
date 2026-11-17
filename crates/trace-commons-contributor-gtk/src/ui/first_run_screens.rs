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
    use crate::first_run::tools::{Found, Tools};
    use std::{cell::RefCell, rc::Rc};

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
