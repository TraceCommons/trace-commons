//! What a run of transfers looks like at a moment, drawn with cairo.
//!
//! A port of `MapTransferDrawing` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/MapToolPile.swift`: the
//! traces as glowing orbs on the lines, the pulse as each vanishes at the
//! pad, the rocket on the pad, its lift-off and flight to the library, the
//! dock's ring, and the light going once round a coin's rim. Drawn under
//! the pile, as on the Mac, except the rim light, which is drawn over it.
//!
//! Colours are the Mac's own map tokens (`Palette::map`): the shared and
//! kept orbs, the rocket's steel, accent and porthole, the dock's ring.
//!
//! Deliberate platform differences:
//! - The orbs' and the rocket's glow are soft radial halos; cairo has no
//!   blur filter.
//! - Only the launch style is drawn (see `transfers`), so a contributed
//!   trace never rises to an orb here.

use std::f64::consts::{FRAC_PI_2, TAU};

use gtk::cairo::Context;
use gtk::gdk::RGBA;

use super::map::{Palette, circle, source};
use super::map_scene::{Arc, ArcStyle, MapGeometry, Point, Scene, ToolStack};
use super::transfers::{Action, Leg, MapStyle, Run, ease, point_on};

/// A run's colour (owner, 2026-10-20): blue for traces being shared,
/// orange for traces kept on this computer; a dismissed trace is not
/// shared, so it takes the kept colour (`MapTransferDrawing.glowToken`).
pub fn glow(action: Option<Action>, palette: &Palette) -> RGBA {
    if action == Some(Action::Contribute) {
        palette.map.orb_shared
    } else {
        palette.map.orb_kept
    }
}

/// The rocket's length along its flight, in design units: the pad's size.
pub const ROCKET_LENGTH: f64 = 44.0;

/// Where the rocket stands on the pad.
fn rocket_rest(hub: Point) -> Point {
    Point::new(hub.x, hub.y - 5.0 - ROCKET_LENGTH / 2.0)
}

fn hub(scene: &Scene) -> Point {
    scene
        .nodes
        .iter()
        .find(|n| n.id == "hub")
        .map_or_else(|| Scene::hub(scene.space), |n| n.at)
}

fn library(scene: &Scene) -> Point {
    scene
        .nodes
        .iter()
        .find(|n| n.id == "library")
        .map_or_else(|| Scene::library(scene.space), |n| n.at)
}

/// The arc a folder's traces ride: the one leaving its dot.
pub fn folder_arc(folder: &str, scene: &Scene) -> Option<Arc> {
    let node = scene
        .nodes
        .iter()
        .find(|n| n.id == format!("folder:{folder}"))?;
    let start = Point::new(node.at.x + super::map_scene::FOLDER_RADIUS + 1.0, node.at.y);
    scene
        .arcs
        .iter()
        .find(|arc| (arc.from.x - start.x).abs() < 0.5 && (arc.from.y - start.y).abs() < 0.5)
        .copied()
}

/// Text the canvas draws for the run, after the shapes: the dock's "+N".
pub struct DockLabel {
    pub at: Point,
    pub text: String,
}

/// Draws the run at `t`; returns the dock's label while it shows.
pub fn draw(
    cr: &Context,
    scene: &Scene,
    run: &Run,
    t: f64,
    fit: &MapGeometry,
    palette: &Palette,
) -> Option<DockLabel> {
    let stack = scene.stack.as_ref()?;
    let s = fit.scale;
    let contributing = run.action() == Action::Contribute;
    let colour = glow(Some(run.action()), palette);
    let hub = hub(scene);
    let library = library(scene);
    let port = stack.port_right();
    let out_arc = scene
        .arcs
        .iter()
        .rev()
        .find(|arc| arc.from == port)
        .copied();
    let rest = rocket_rest(hub);
    // On the standard map a contributed trace rises from the laptop's
    // screen to the orb's base, and the library takes it in.
    let screen = Point::new(hub.x, hub.y - super::map::LAPTOP_SCREEN_TOP);
    let orb_base = Point::new(library.x, library.y + orb_radius());
    let pulse_at = if run.rises() {
        library
    } else if contributing {
        rest
    } else {
        hub
    };

    for (index, transfer) in run.transfers.iter().enumerate() {
        match run.leg(index, t) {
            Leg::Waiting | Leg::Orbit(_) => {}
            Leg::Ascent(p) => {
                if p < 0.3 {
                    pulse(cr, fit.point(screen), p / 0.3, &colour, s * 0.6);
                }
                rise(cr, screen, orb_base, p, &colour, fit);
            }
            Leg::Inbound(p) => {
                if let Some(arc) = folder_arc(&transfer.folder, scene) {
                    orb(cr, fit.point(point_on(&arc, ease(p))), &colour, s);
                }
            }
            Leg::Outbound(p) => {
                if let Some(arc) = out_arc.as_ref() {
                    orb(cr, fit.point(point_on(arc, ease(p))), &colour, s);
                }
            }
            Leg::Arrived(since) => {
                if since < Run::PULSE {
                    pulse(cr, fit.point(pulse_at), since / Run::PULSE, &colour, s);
                }
            }
        }
    }

    if contributing && run.style == MapStyle::Launch {
        return rocket(cr, run, t, rest, library, fit, palette);
    }
    if run.rises() {
        return dock(
            cr,
            fit.point(library),
            t - run.all_arrived(),
            s,
            run,
            palette,
        );
    }
    None
}

/// The library orb's radius on the standard map, in design units
/// (`MapTransferDrawing.orbRadius`).
fn orb_radius() -> f64 {
    super::orb::FRAME / 2.0 * (1.0 - super::orb::INSET)
}

/// A trace rising from the laptop's screen to the orb's base: slowly away,
/// then faster, with a short fading trail (`MapTransferDrawing.rise`).
fn rise(cr: &Context, start: Point, end: Point, progress: f64, colour: &RGBA, fit: &MapGeometry) {
    let s = fit.scale;
    for step in (0..=3).rev() {
        let lag = f64::from(step) * 0.045;
        let along = (progress - lag).max(0.0).powf(1.7);
        let at = Point::new(
            start.x + (end.x - start.x) * along,
            start.y + (end.y - start.y) * along,
        );
        // Each step of the trail faded as a whole, as the Mac's layer is.
        cr.push_group();
        orb(
            cr,
            fit.point(at),
            colour,
            s * (1.0 - f64::from(step) * 0.18),
        );
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(1.0 - f64::from(step) * 0.28);
    }
}

/// The light going once round the rim of each coin a trace is in,
/// counter-clockwise from its base.
pub fn rim_lights(
    cr: &Context,
    stack: &ToolStack,
    run: &Run,
    t: f64,
    fit: &MapGeometry,
    palette: &Palette,
) {
    let s = fit.scale;
    let colour = glow(Some(run.action()), palette);
    let order = run.order(&stack.tools, t);
    for (id, p) in run.orbits(t) {
        let Some(index) = order.iter().position(|tool| *tool == id) else {
            continue;
        };
        let centre = fit.point(stack.coin_center(index, false));
        let r = ToolStack::COIN / 2.0 * 1.3 * s;
        let angle = TAU * p;
        let light = Point::new(centre.x + r * angle.sin(), centre.y + r * angle.cos());
        orb(cr, light, &colour, s * 0.75);
    }
}

/// The rocket standing on the pad between contributions, unlit; none on
/// the standard map, which has no rocket.
pub fn resting_rocket(
    cr: &Context,
    scene: &Scene,
    fit: &MapGeometry,
    palette: &Palette,
    style: MapStyle,
) {
    if scene.stack.is_none() || style == MapStyle::Standard {
        return;
    }
    let rest = rocket_rest(hub(scene));
    draw_rocket(
        cr,
        fit.point(rest),
        -FRAC_PI_2,
        fit.scale,
        None,
        None,
        palette,
    );
}

// --- Orbs and pulses -------------------------------------------------------------

fn radial(cr: &Context, centre: Point, radius: f64, stops: &[(f64, &RGBA, f64)]) {
    let gradient =
        gtk::cairo::RadialGradient::new(centre.x, centre.y, 0.0, centre.x, centre.y, radius);
    for (offset, colour, alpha) in stops {
        gradient.add_color_stop_rgba(
            *offset,
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha()) * alpha,
        );
    }
    circle(cr, centre, radius);
    let _ = cr.set_source(&gradient);
    let _ = cr.fill();
}

/// A trace: a white core in its colour, with a soft halo.
pub fn orb(cr: &Context, centre: Point, colour: &RGBA, s: f64) {
    let r = 7.0 * s;
    radial(
        cr,
        centre,
        r * 2.0,
        &[(0.0, colour, 0.45), (1.0, colour, 0.0)],
    );
    let core = Point::new(centre.x - r * 0.2, centre.y - r * 0.25);
    radial(
        cr,
        core,
        r * 1.2,
        &[
            (0.0, &RGBA::WHITE, 1.0),
            (0.5, colour, 0.9),
            (1.0, colour, 0.0),
        ],
    );
}

/// The pulse a trace gives off as it vanishes.
pub fn pulse(cr: &Context, centre: Point, progress: f64, colour: &RGBA, s: f64) {
    let r = (6.0 + 38.0 * progress) * s;
    let alpha = if progress < 0.2 {
        progress / 0.2 * 0.95
    } else {
        0.95 * (1.0 - (progress - 0.2) / 0.8)
    };
    radial(
        cr,
        centre,
        r,
        &[
            (0.0, &RGBA::WHITE, alpha),
            (0.5, colour, alpha * 0.75),
            (1.0, colour, 0.0),
        ],
    );
}

// --- The rocket --------------------------------------------------------------------

/// The flame's colours, as the Mac's map draws them: a warm orange and a
/// near-white core.
fn flame_outer() -> RGBA {
    RGBA::new(1.0, 0.7, 0.36, 1.0)
}

fn flame_inner() -> RGBA {
    RGBA::new(1.0, 0.95, 0.79, 1.0)
}

fn rocket(
    cr: &Context,
    run: &Run,
    t: f64,
    rest: Point,
    library: Point,
    fit: &MapGeometry,
    palette: &Palette,
) -> Option<DockLabel> {
    let s = fit.scale;
    let flight = run.flight(t);
    if let Some(flight) = flight
        && flight >= 1.0
    {
        return dock(
            cr,
            fit.point(library),
            t - run.lift_off() - Run::FLIGHT,
            s,
            run,
            palette,
        );
    }
    // Up off the pad, then over to the library.
    let path = Arc {
        from: rest,
        control1: Point::new(rest.x, rest.y - 118.0),
        control2: Point::new(library.x, library.y + 110.0),
        to: library,
        style: ArcStyle::Quiet,
        dim: 1.0,
    };
    let progress = flight.unwrap_or(0.0);
    let along = Run::climb(progress);
    let at = point_on(&path, along);
    let ahead = point_on(&path, (along + 0.01).min(1.0));
    let mut angle = (ahead.y - at.y).atan2(ahead.x - at.x);
    if along >= 0.999 || ahead == at {
        angle = -FRAC_PI_2;
    }
    let mut centre = fit.point(at);
    // Ignition: a shake on the pad before it moves.
    if flight.is_some() && progress < 0.26 {
        centre.x += (t * 90.0).sin() * 0.7 * s;
    }
    let charge = run.arrived(t).min(4);
    let (glow_colour, glow_radius) = if flight.is_some() {
        (flame_outer(), 8.0 * s)
    } else if charge > 0 {
        (palette.map.orb_shared, (5.0 + charge as f64 * 4.0) * s)
    } else {
        (flame_outer().with_alpha(0.5), 5.0 * s)
    };
    if flight.is_some() && progress < 0.6 {
        let base = fit.point(Point::new(rest.x, rest.y + ROCKET_LENGTH / 2.0));
        smoke(cr, base, progress / 0.6, s);
    }
    draw_rocket(
        cr,
        centre,
        angle,
        s,
        Some((&glow_colour, glow_radius)),
        flight.map(|f| 0.35 + 1.55 * f),
        palette,
    );
    None
}

/// The rocket at `centre`, nose along `angle`; `burn` is the flame's size,
/// none while it stands. Its body is outlined so it holds on a light map
/// as well as a dark one. Drawn in its own 30 by 18 units, nose right.
fn draw_rocket(
    cr: &Context,
    centre: Point,
    angle: f64,
    s: f64,
    glow: Option<(&RGBA, f64)>,
    burn: Option<f64>,
    palette: &Palette,
) {
    if let Some((colour, radius)) = glow
        && radius > 0.0
    {
        radial(
            cr,
            centre,
            radius * 2.5,
            &[(0.0, colour, 0.5), (1.0, colour, 0.0)],
        );
    }
    let _ = cr.save();
    cr.translate(centre.x, centre.y);
    cr.rotate(angle);
    let k = ROCKET_LENGTH * s / 30.0;
    cr.scale(k, k);
    cr.translate(-15.0, -9.0);
    if let Some(burn) = burn {
        let _ = cr.save();
        cr.translate(9.0, 9.0);
        cr.scale(burn, burn);
        cr.translate(-9.0, -9.0);
        flame_path(cr);
        source(cr, &flame_outer(), 1.0);
        let _ = cr.fill();
        flame_core(cr);
        source(cr, &flame_inner(), 1.0);
        let _ = cr.fill();
        let _ = cr.restore();
    }
    let outline = palette.map.status_off;
    let accent = palette.map.purple_soft;
    cr.set_line_width(0.9);
    nozzle_path(cr);
    source(cr, &outline, 1.0);
    let _ = cr.fill_preserve();
    let _ = cr.stroke();
    fins_path(cr);
    source(cr, &accent, 1.0);
    let _ = cr.fill_preserve();
    source(cr, &outline, 1.0);
    let _ = cr.stroke();
    body_path(cr);
    source(cr, &RGBA::new(0.95, 0.95, 0.95, 1.0), 1.0);
    let _ = cr.fill();
    nose_path(cr);
    source(cr, &accent, 1.0);
    let _ = cr.fill();
    body_path(cr);
    source(cr, &outline, 1.0);
    let _ = cr.stroke();
    // The porthole.
    circle(cr, Point::new(18.6, 9.0), 2.0);
    source(cr, &palette.map.blue, 1.0);
    let _ = cr.fill_preserve();
    source(cr, &outline, 1.0);
    let _ = cr.stroke();
    let _ = cr.restore();
}

/// The body and its nose, one outline.
fn body_path(cr: &Context) {
    cr.new_path();
    cr.move_to(10.0, 5.6);
    cr.line_to(20.0, 5.6);
    cr.curve_to(24.5, 5.6, 28.0, 7.0, 29.5, 9.0);
    cr.curve_to(28.0, 11.0, 24.5, 12.4, 20.0, 12.4);
    cr.line_to(10.0, 12.4);
    cr.close_path();
}

/// The nose cone's colour, inside the body's outline.
fn nose_path(cr: &Context) {
    cr.new_path();
    cr.move_to(23.5, 6.1);
    cr.curve_to(26.0, 6.6, 28.3, 7.6, 29.5, 9.0);
    cr.curve_to(28.3, 10.4, 26.0, 11.4, 23.5, 11.9);
    // The quadratic back to the start, as a cubic.
    cr.curve_to(24.1, 10.0, 24.1, 8.0, 23.5, 6.1);
    cr.close_path();
}

/// Two fins swept back from the body to tips by the tail.
fn fins_path(cr: &Context) {
    cr.new_path();
    cr.move_to(17.0, 5.6);
    cr.curve_to(15.0, 3.2, 12.5, 1.2, 10.5, 0.8);
    cr.curve_to(9.4, 0.6, 9.6, 3.5, 10.0, 5.6);
    cr.close_path();
    cr.move_to(17.0, 12.4);
    cr.curve_to(15.0, 14.8, 12.5, 16.8, 10.5, 17.2);
    cr.curve_to(9.4, 17.4, 9.6, 14.5, 10.0, 12.4);
    cr.close_path();
}

/// The nozzle, flaring from the tail towards the flame.
fn nozzle_path(cr: &Context) {
    cr.new_path();
    cr.move_to(10.4, 7.2);
    cr.line_to(8.0, 6.4);
    cr.line_to(8.0, 11.6);
    cr.line_to(10.4, 10.8);
    cr.close_path();
}

fn flame_path(cr: &Context) {
    cr.new_path();
    cr.move_to(0.0, 9.0);
    cr.curve_to(4.0, 5.5, 7.0, 6.5, 9.0, 9.0);
    cr.curve_to(7.0, 11.5, 4.0, 12.5, 0.0, 9.0);
    cr.close_path();
}

fn flame_core(cr: &Context) {
    cr.new_path();
    cr.move_to(3.0, 9.0);
    cr.curve_to(5.0, 7.5, 7.0, 8.0, 9.0, 9.0);
    cr.curve_to(7.0, 10.0, 5.0, 10.5, 3.0, 9.0);
    cr.close_path();
}

fn smoke(cr: &Context, base: Point, progress: f64, s: f64) {
    let grey = RGBA::new(0.86, 0.86, 0.86, 1.0);
    for (offset, delay) in [(0.0, 0.0), (-14.0, 0.15)] {
        let p = (progress - delay).max(0.0) / (1.0 - delay);
        if p <= 0.0 {
            continue;
        }
        let r = (6.0 + 30.0 * p) * s;
        let centre = Point::new(base.x + offset * s, base.y - 4.0 * s);
        radial(
            cr,
            centre,
            r,
            &[(0.0, &grey, 0.5 * (1.0 - p)), (1.0, &grey, 0.0)],
        );
    }
}

/// The library lights as the rocket docks, and says how many arrived.
fn dock(
    cr: &Context,
    centre: Point,
    since: f64,
    s: f64,
    run: &Run,
    palette: &Palette,
) -> Option<DockLabel> {
    if since < 0.0 {
        return None;
    }
    if since < 1.1 {
        let p = since / 1.1;
        circle(cr, centre, (28.0 + 26.0 * p) * s);
        cr.set_line_width(3.0 * s);
        source(cr, &palette.map.blue, 0.75 * (1.0 - p));
        let _ = cr.stroke();
    }
    (since < Run::LINGER).then(|| DockLabel {
        at: Point::new(centre.x + 40.0 * s, centre.y - 30.0 * s),
        text: format!("+{}", run.transfers.len()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::monitor::map_scene::{CommonsGate, Contributions, ScreenState};
    use crate::ui::monitor::traces_tree::TracesTree;
    use crate::ui::monitor::transfers::MapTransfer;

    fn scene() -> Scene {
        let entries = vec![crate::model::QueueEntry {
            entry_id: "e".into(),
            project_id: "p".into(),
            project_label: "p".into(),
            source: "codex".into(),
            state: "pending".into(),
            ..Default::default()
        }];
        let tree = TracesTree::build(&entries, &[], None);
        Scene::traces(
            &tree,
            &CommonsGate::with_state(ScreenState::Ready),
            &Contributions::unread(),
            None,
            &Default::default(),
            super::super::map_scene::SIZE,
        )
    }

    /// A folder's traces ride the arc leaving its dot; one not on the map
    /// rides none.
    #[test]
    fn a_folders_trace_rides_the_arc_from_its_dot() {
        let scene = scene();
        let arc = folder_arc("p", &scene).expect("the folder's arc");
        let dot = scene.nodes.iter().find(|n| n.id == "folder:p").unwrap();
        assert!(arc.from.x > dot.at.x, "it leaves the dot's right edge");
        assert!((arc.from.y - dot.at.y).abs() < 0.5);
        assert!(folder_arc("elsewhere", &scene).is_none());
    }

    /// The rocket stands on the pad, above this computer.
    #[test]
    fn the_rocket_stands_on_the_pad() {
        let scene = scene();
        let rest = rocket_rest(hub(&scene));
        assert_eq!(rest.x, hub(&scene).x);
        assert!(rest.y < hub(&scene).y);
        assert!(library(&scene).y < rest.y, "the library is above it");
    }

    /// A contribution is drawn in the shared orb's blue, a keep or a
    /// dismissal in the kept orb's orange (`MapTransferDrawing.glowToken`).
    #[test]
    fn a_run_takes_its_actions_colour() {
        let palette = Palette::fixed();
        assert_eq!(
            glow(Some(Action::Contribute), &palette),
            palette.map.orb_shared
        );
        assert_eq!(glow(Some(Action::Keep), &palette), palette.map.orb_kept);
        assert_eq!(glow(Some(Action::Dismiss), &palette), palette.map.orb_kept);
        assert_eq!(glow(None, &palette), palette.map.orb_kept);
        let run = Run::new(
            vec![MapTransfer::new("1", "p", "codex", Action::Keep)],
            0.0,
            MapStyle::Launch,
        );
        assert_eq!(run.action(), Action::Keep);
    }

    /// Drawn alpha at `(x, y)` (design units at scale 1) of `draw`.
    fn alpha_at(points: &[Point], draw: impl Fn(&Context)) -> Vec<u8> {
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1400, 1400).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            draw(&cr);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        points
            .iter()
            .map(|p| data[p.y as usize * stride + p.x as usize * 4 + 3])
            .collect()
    }

    /// On the standard map there is no rocket (`MapTransferDrawing`): none
    /// stands on the hub between runs, and a contribution rises on its own
    /// from the laptop's screen (36 above this computer) to the orb's base,
    /// then the library takes it in with a pulse and says "+N"; on the
    /// rocket map the rocket stands on the pad.
    #[test]
    fn on_the_standard_map_traces_rise_to_the_orb_and_no_rocket_stands() {
        let scene = scene();
        let fit = MapGeometry::new(super::super::map_scene::SIZE, 1.0, None, scene.space);
        let palette = Palette::fixed();
        let rest = fit.point(rocket_rest(hub(&scene)));
        let standing = |style| {
            alpha_at(&[rest], |cr| {
                resting_rocket(cr, &scene, &fit, &palette, style)
            })[0]
        };
        assert!(standing(MapStyle::Launch) > 0, "the rocket map's rocket");
        assert_eq!(
            standing(MapStyle::Standard),
            0,
            "a rocket on the standard map"
        );

        let transfer = MapTransfer::new("1", "p", "codex", Action::Contribute);
        let run = Run::new(vec![transfer], 0.0, MapStyle::Standard);
        assert!(run.rises());
        // Halfway up its rise: between the screen and the orb's base.
        let into_ascent = Run::INBOUND + Run::ORBIT + Run::OUTBOUND + Run::ASCENT * 0.6;
        let hub = hub(&scene);
        let library = library(&scene);
        let screen = Point::new(hub.x, hub.y - 36.0);
        let base = Point::new(library.x, library.y + 104.0 / 2.0 * (1.0 - 0.29));
        let along = 0.6_f64.powf(1.7);
        let at = fit.point(Point::new(
            screen.x + (base.x - screen.x) * along,
            screen.y + (base.y - screen.y) * along,
        ));
        let rising = alpha_at(&[at, rest], |cr| {
            draw(cr, &scene, &run, into_ascent, &fit, &palette);
        });
        assert!(rising[0] > 100, "no trace rising at {at:?}: {}", rising[0]);
        assert_eq!(rising[1], 0, "a rocket on the standard map's pad");
        // Once it has arrived the library says how many.
        let after = run.all_arrived() + 0.2;
        let label = std::cell::RefCell::new(None);
        alpha_at(&[], |cr| {
            label.replace(draw(cr, &scene, &run, after, &fit, &palette));
        });
        let label = label.take().expect("the library's +N");
        assert_eq!(label.text, "+1");
        let centre = fit.point(library);
        assert_eq!(label.at, Point::new(centre.x + 40.0, centre.y - 30.0));
    }
}
