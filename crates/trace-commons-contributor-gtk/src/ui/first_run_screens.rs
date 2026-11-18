//! The first run's quieter screens, drawn as the Mac draws them: Welcome
//! with Get started and the agent prompt (`WelcomeScreen`,
//! `AgentConfigurationScreen`), the Private AI diagram and its count
//! (`PrivateAIScreen`), and the two diagrams Traces and Uses stand under.
//! Every word is the core's; the diagrams are drawn, not written.
use adw::prelude::*;
use trace_commons_contributor::first_run_copy::first_run_copy;

use super::lift::Lift;
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
///
/// Each line rises into place on Welcome's clock (`WelcomeScreen`): the
/// title a word at a time, then the line under it, then the buttons. The
/// lines and their delays come back for the stage to raise
/// ([`super::first_run_stage_view::Stage::set_rises`]); the title is one
/// heading to assistive tech, as the Mac's is.
pub(super) fn welcome(
    on_start: impl Fn() + 'static,
    on_agent: impl Fn() + 'static,
) -> (gtk::Box, Vec<(Lift, f64)>) {
    use super::first_run_stage::welcome as layout;
    let copy = first_run_copy();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.set_margin_start(32);
    root.set_margin_end(32);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    let mut rises = Vec::new();
    let title = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .accessible_role(gtk::AccessibleRole::Heading)
        .build();
    title.update_property(&[gtk::accessible::Property::Label(copy.welcome.title)]);
    for (index, word) in layout::words(copy.welcome.title).iter().enumerate() {
        let label = gtk::Label::builder()
            .label(word.as_str())
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        label.add_css_class("tc-flow-title");
        let lift = Lift::new(&label);
        title.append(&lift);
        rises.push((lift, layout::word_delay(index)));
    }
    root.append(&title);
    let body = Lift::new(&body_label(copy.welcome.body));
    root.append(&body);
    rises.push((body, layout::BODY_DELAY));
    let start = gtk::Button::with_label(copy.welcome.start);
    start.add_css_class("suggested-action");
    start.add_css_class("pill");
    start.connect_clicked(move |_| on_start());
    let start = Lift::new(&start);
    start.set_halign(gtk::Align::Start);
    root.append(&start);
    rises.push((start, layout::BUTTON_DELAY));
    let agent = gtk::Button::with_label(copy.agent_configuration.action);
    agent.connect_clicked(move |_| on_agent());
    let agent = Lift::new(&agent);
    agent.set_halign(gtk::Align::Start);
    root.append(&agent);
    rises.push((agent, layout::BUTTON_DELAY));
    (root, rises)
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

/// The Private AI routes' words: each tool that can connect, and the
/// destination's pill. The routes themselves, the tiles and the N are the
/// stage's (`first_run_stage`), drawn above the title as the Mac does.
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

/// The Uses flow's captions: this machine, the review gate and the
/// commons, under the stage's flow (`first_run_stage`), which draws the
/// line, the gate, the orb and the N. Each is centred under its node, as
/// the Mac sets each label 160 points wide at x 210, 430 and 650 of its
/// canvas (`UsesFlow`), at the stage's scale (`scale`). The canvas is
/// centred on the page, as the stage centres it on the graphic's slot; the
/// captions take no width of their own, so the outer two may reach past
/// the column as the Mac's reach past its card. The gate reads the core's
/// "You review" on Ask and the mode table's Automatic on Automatic.
pub(super) fn uses_flow(
    gate: &str,
    scale: impl Fn() -> f64 + 'static,
) -> (gtk::Overlay, std::rc::Rc<dyn Fn()>) {
    use super::first_run_stage::{CANVAS, uses};
    let copy = first_run_copy();
    let root = gtk::Overlay::new();
    let room = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_child(Some(&room));
    let mut labels = Vec::new();
    // The Mac's "This Mac", for any computer (owner, 2026-11-17).
    for (text, x) in [
        (copy.uses.this_computer, uses::MAC_X),
        (gate, uses::GATE_X),
        (copy.uses.the_commons, uses::COMMONS_X),
    ] {
        let label = gtk::Label::builder()
            .label(text)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        label.add_css_class("tc-meta");
        root.add_overlay(&label);
        labels.push((label, x));
    }
    let width = move |scale: f64| (uses::CAPTION_WIDTH * scale).round().max(1.0) as i32;
    let scale = std::rc::Rc::new(scale);
    // The room under the captions: as tall as the tallest at its width,
    // set again whenever the scale changes (`relayout`).
    let relayout: std::rc::Rc<dyn Fn()> = {
        let labels: Vec<gtk::Label> = labels.iter().map(|(l, _)| l.clone()).collect();
        let (scale, room) = (scale.clone(), room.clone());
        std::rc::Rc::new(move || {
            let w = width(scale());
            let tall = labels
                .iter()
                .map(|l| l.measure(gtk::Orientation::Vertical, w).1)
                .max()
                .unwrap_or(0);
            if room.height_request() != tall {
                room.set_height_request(tall);
            }
            room.queue_resize();
        })
    };
    relayout();
    root.connect_get_child_position(move |root, child| {
        let (label, x) = labels
            .iter()
            .find(|(l, _)| l.upcast_ref::<gtk::Widget>() == child)?;
        let s = scale();
        let w = width(s);
        let h = label.measure(gtk::Orientation::Vertical, w).1;
        let centre = f64::from(root.width()) / 2.0 + (x - CANVAS.0 / 2.0) * s;
        Some(gtk::gdk::Rectangle::new(
            (centre - f64::from(w) / 2.0).round() as i32,
            0,
            w,
            h,
        ))
    });
    (root, relayout)
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
        let (root, rises) = welcome(|| {}, || {});
        let labels = buttons(&root);
        // Every word of the title, the line under it and both buttons rise.
        let words = super::super::first_run_stage::welcome::words(copy.welcome.title);
        assert_eq!(rises.len(), words.len() + 3);
        let delays: Vec<f64> = rises.iter().map(|(_, d)| *d).collect();
        assert_eq!(&delays[..3], &[1.05, 1.14, 1.23][..words.len().min(3)]);
        assert_eq!(&delays[words.len()..], &[1.45, 1.8, 1.8]);
        let heading = root.first_child().unwrap();
        assert_eq!(heading.accessible_role(), gtk::AccessibleRole::Heading);
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
        let (flow, _) = uses_flow(copy.uses.you_review, || 1.0);
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

    fn settle(context: &glib::MainContext, millis: u64) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        while std::time::Instant::now() < end {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Each Uses caption is centred under its node, at x 210, 430 and 650
    /// of the canvas centred on the page, at two scales.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_uses_captions_sit_under_their_nodes() {
        let context = init();
        let _owner = context.acquire().unwrap();
        let copy = first_run_copy();
        for (width, scale) in [(900, 1.0), (680, 680.0 / 860.0)] {
            let (flow, relayout) = uses_flow(copy.uses.you_review, move || scale);
            let window = gtk::Window::builder().default_width(width).build();
            let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
            page.append(&flow);
            window.set_child(Some(&page));
            window.present();
            relayout();
            settle(&context, 200);
            let centre = f64::from(flow.width()) / 2.0;
            let labels: Vec<gtk::Label> = descendants(flow.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .collect();
            for (text, x) in [
                (copy.uses.this_computer, 210.0),
                (copy.uses.you_review, 430.0),
                (copy.uses.the_commons, 650.0),
            ] {
                let label = labels.iter().find(|l| l.text() == text).unwrap();
                let b = label.compute_bounds(&flow).unwrap();
                let got = f64::from(b.x()) + f64::from(b.width()) / 2.0;
                let want = centre + (x - 430.0) * scale;
                assert!(
                    (got - want).abs() <= 1.0,
                    "{text} at {width}: {got} != {want}"
                );
                assert_eq!(b.width().round() as i32, (160.0 * scale).round() as i32);
            }
            assert!(flow.height() > 0, "the captions take their room");
            window.destroy();
        }
    }

    /// The marker check is the real check button, drawn over: its stroke
    /// ticks only while it moves and only while shown; with animations off
    /// it does not tick at all.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_marker_check_ticks_only_while_its_stroke_moves() {
        use super::super::first_run_marker_check::MarkerCheck;
        let context = init();
        let _owner = context.acquire().unwrap();
        super::super::style::install();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let mark = MarkerCheck::new(gtk::CheckButton::with_label("x"), None);
        let window = gtk::Window::new();
        window.set_child(Some(&mark.root));
        window.present();
        settle(&context, 100);
        assert!(!mark.ticking(), "at rest");
        assert!(mark.check.has_css_class("tc-marker-check"));
        mark.check.set_active(true);
        assert!(mark.ticking(), "drawing on");
        settle(&context, 500);
        assert!(!mark.ticking(), "stops once drawn");
        mark.check.set_active(false);
        assert!(mark.ticking());
        window.set_visible(false);
        settle(&context, 50);
        assert!(!mark.ticking(), "no tick while hidden");
        window.present();
        settings.set_gtk_enable_animations(false);
        settle(&context, 50);
        mark.check.set_active(true);
        assert!(!mark.ticking(), "no tick with animations off");
        settings.set_gtk_enable_animations(previous);
        window.destroy();
    }

    /// The Customize card rises from where Uses' card was: drawn there at
    /// first, then home, faded in; nothing moves with animations off.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn the_customize_card_rises_to_its_place() {
        use super::super::lift::Lift;
        let context = init();
        let _owner = context.acquire().unwrap();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let above = gtk::Box::new(gtk::Orientation::Vertical, 0);
        above.set_size_request(-1, 200);
        page.append(&above);
        let lift = Lift::new(&gtk::Label::new(Some("x")));
        page.append(&lift);
        let window = gtk::Window::builder()
            .default_width(400)
            .default_height(400)
            .build();
        window.set_child(Some(&page));
        window.present();
        settle(&context, 100);
        // The card was 200 points lower; what was above it gives way.
        above.set_visible(false);
        lift.slide_from(&page, 200.0);
        // The first frames lay it out, then it is drawn where it was: wait
        // for that frame, however slowly the compositor paints.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while lift.offset() == 0.0 && std::time::Instant::now() < end {
            settle(&context, 2);
        }
        assert!(
            lift.offset() > 100.0,
            "starts where it was: {}",
            lift.offset()
        );
        assert!(lift.opacity() < 0.5);
        settle(&context, 1600);
        assert!(!lift.sliding());
        assert_eq!(lift.offset(), 0.0);
        assert_eq!(lift.opacity(), 1.0);
        settings.set_gtk_enable_animations(false);
        lift.slide_from(&page, 200.0);
        assert!(!lift.sliding());
        assert_eq!(lift.offset(), 0.0);
        settings.set_gtk_enable_animations(previous);
        window.destroy();
    }
}
