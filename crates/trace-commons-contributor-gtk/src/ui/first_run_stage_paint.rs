//! The first run's stage, drawn with cairo: a port of the Mac's views in
//! `FirstRunStage.swift` (`FirstRunBackdrop`, `FirstRunHub`,
//! `FirstRunToolFace`, `FirstRunOrb`, the step thread's stretches),
//! `GlassBrandMarkDrawing` and each step's graphic (`ToolsRing`, the
//! Private AI routes, Traces' streams and ring, `UsesFlow`, Join's orbit).
//!
//! Every number is [`super::first_run_stage`]'s; this file only paints. It
//! draws no words: the stage is decorative, as the Mac hides it from
//! assistive tech. The one exception is a logo-less tool's two-letter mark
//! (Gemini CLI, Cline), which the Mac draws on its tile too.
//!
//! SwiftUI's blurs have no cairo counterpart. Where the Mac blurs a soft
//! shape, this draws the same shape with a gradient that already falls to
//! nothing, or a few widening translucent passes; the comparison renders
//! measure what that leaves.

#![allow(dead_code)] // TEMP until the view lands
use std::f64::consts::{PI, TAU};

use gtk::cairo;

use super::first_run_stage::{self as stage, Frame, Point, Segment};
use super::glyph::Artwork;
use super::style::brand_tokens::first_run as tokens;
use crate::first_run::Sharing;

/// An sRGB colour with alpha, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f64, pub f64, pub f64, pub f64);

impl Rgba {
    fn from_token((hex, alpha): tokens::Rgba) -> Self {
        let channel = |i: usize| f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)) / 255.0;
        Self(channel(1), channel(3), channel(5), alpha)
    }
    /// The colour at `alpha` times its own.
    pub fn alpha(self, alpha: f64) -> Self {
        Self(self.0, self.1, self.2, self.3 * alpha)
    }
    /// The colour at exactly `alpha` (SwiftUI's `.opacity` on a token).
    pub fn with(self, alpha: f64) -> Self {
        Self(self.0, self.1, self.2, alpha)
    }
}

/// The stage's colours in one scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub scene_base: Rgba,
    pub scene_warm: Rgba,
    pub glow: Rgba,
    pub beam: Rgba,
    pub beam_core: Rgba,
    pub card: Rgba,
    pub card_solid: Rgba,
    pub tile: Rgba,
    pub tile_shadow: Rgba,
    pub rule: Rgba,
    pub approve: Rgba,
    pub approve_deep: Rgba,
    pub hero_ink: Rgba,
    pub well: Rgba,
    pub text_primary: Rgba,
    pub tool_claude: Rgba,
    pub tool_codex: Rgba,
    pub tool_antigravity: Rgba,
    pub tool_open_code: Rgba,
}

impl Palette {
    /// The scheme the stage draws in. The first run is dark only on the Mac
    /// (Ron, 2026-11-05); light is kept for the comparison renders.
    pub fn scheme(dark: bool) -> Self {
        macro_rules! pick {
            ($name:ident) => {
                Rgba::from_token(if dark {
                    tokens::dark::$name
                } else {
                    tokens::light::$name
                })
            };
        }
        Self {
            dark,
            scene_base: pick!(SCENE_BASE),
            scene_warm: pick!(SCENE_WARM),
            glow: pick!(FIRST_RUN_GLOW),
            beam: pick!(FIRST_RUN_BEAM),
            beam_core: pick!(FIRST_RUN_BEAM_CORE),
            card: pick!(FIRST_RUN_CARD),
            card_solid: pick!(FIRST_RUN_CARD_SOLID),
            tile: pick!(FIRST_RUN_TILE),
            tile_shadow: pick!(FIRST_RUN_TILE_SHADOW),
            rule: pick!(FIRST_RUN_RULE),
            approve: pick!(FIRST_RUN_APPROVE),
            approve_deep: pick!(FIRST_RUN_APPROVE_DEEP),
            hero_ink: pick!(HERO_INK),
            well: pick!(WELL_FILL),
            text_primary: pick!(TEXT_PRIMARY),
            tool_claude: pick!(TOOL_CLAUDE),
            tool_codex: pick!(TOOL_CODEX),
            tool_antigravity: pick!(TOOL_ANTIGRAVITY),
            tool_open_code: pick!(TOOL_OPEN_CODE),
        }
    }
}

fn source(cr: &cairo::Context, c: Rgba) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

fn stop(g: &cairo::Gradient, at: f64, c: Rgba) {
    g.add_color_stop_rgba(at, c.0, c.1, c.2, c.3);
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// A soft shadow under a rounded rectangle: SwiftUI's `.shadow(radius:)`,
/// as a few widening translucent passes.
fn soft_shadow(cr: &cairo::Context, rect: (f64, f64, f64, f64), corner: f64, blur: f64, dy: f64, c: Rgba) {
    if c.3 <= 0.0 || blur <= 0.0 {
        return;
    }
    let passes = 6;
    for i in 0..passes {
        let grow = blur * (i as f64 + 1.0) / passes as f64;
        rounded(
            cr,
            rect.0 - grow,
            rect.1 - grow + dy,
            rect.2 + 2.0 * grow,
            rect.3 + 2.0 * grow,
            corner + grow,
        );
        source(cr, c.alpha(1.0 / passes as f64 * 0.9));
        let _ = cr.fill();
    }
}

/// A round glow `radius` across at `center`: a radial fall-off from `c`.
fn glow_disc(cr: &cairo::Context, center: Point, radius: f64, c: Rgba) {
    let g = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, radius);
    stop(&g, 0.0, c);
    stop(&g, 1.0, c.with(0.0));
    let _ = cr.set_source(&g);
    cr.arc(center.0, center.1, radius, 0.0, TAU);
    let _ = cr.fill();
}

// ---- The scene -----------------------------------------------------------

/// The navy base every step shares, with the brand-blue glow breathing from
/// the top-left corner (`FirstRunBackdrop`), over `w` by `h`.
pub fn backdrop(cr: &cairo::Context, w: f64, h: f64, p: &Palette, phase: f64) {
    let _ = cr.save();
    cr.rectangle(0.0, 0.0, w, h);
    cr.clip();
    let reach = w.max(h) * 0.9;
    let base = cairo::RadialGradient::new(w / 2.0, 0.0, 0.0, w / 2.0, 0.0, reach);
    stop(&base, 0.0, p.scene_warm);
    stop(&base, 1.0, p.scene_base);
    let _ = cr.set_source(&base);
    let _ = cr.paint();
    let (strong, soft) = if p.dark { (0.5, 0.14) } else { (0.3, 0.08) };
    // The elliptical glow: centred on the top-left corner, 1.2 widths by
    // 0.95 heights across.
    let _ = cr.save();
    cr.scale(1.2 * w, 0.95 * h);
    let corner = cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
    stop(&corner, 0.0, p.glow.with(strong));
    stop(&corner, 0.42, p.glow.with(soft));
    stop(&corner, 0.75, p.glow.with(0.0));
    let _ = cr.set_source(&corner);
    cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
    let _ = cr.fill();
    let _ = cr.restore();
    // The breathing ellipse: 640 by 580 from (-140, -140), scaled from its
    // top-left corner, its gradient a 300-point circle.
    let scale = 0.9 + 0.25 * phase;
    let opacity = 0.6 + 0.4 * phase;
    let _ = cr.save();
    cr.translate(-140.0, -140.0);
    cr.scale(scale, scale);
    let g = cairo::RadialGradient::new(320.0, 290.0, 0.0, 320.0, 290.0, 300.0);
    stop(&g, 0.0, p.glow.with(strong * opacity));
    stop(&g, 1.0, p.glow.with(0.0));
    let _ = cr.set_source(&g);
    let _ = cr.save();
    cr.translate(320.0, 290.0);
    cr.scale(320.0, 290.0);
    cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
    let _ = cr.restore();
    let _ = cr.fill();
    let _ = cr.restore();
    let _ = cr.restore();
}

// ---- The step thread -----------------------------------------------------

/// One stretch of the thread, `width` across with its top-left at `origin`:
/// the dotted rule, and over it the glowing stretch, `grown` (0 to 1) of
/// its width, lit at its lead end when it leads.
pub fn thread_segment(cr: &cairo::Context, origin: Point, width: f64, segment: Segment, grown: f64, p: &Palette) {
    let (x, y) = origin;
    cr.set_line_width(2.0);
    cr.set_dash(&[2.0, 4.0], 0.0);
    cr.set_line_cap(cairo::LineCap::Butt);
    source(cr, p.rule);
    cr.move_to(x, y + 1.5);
    cr.line_to(x + width, y + 1.5);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let fill = width * grown.clamp(0.0, 1.0);
    if fill <= 0.0 {
        return;
    }
    // Its glow (`.shadow(color: glow 0.35, radius: 4 or 5)`).
    let blur = if segment == Segment::Leading { 5.0 } else { 4.0 };
    soft_shadow(cr, (x, y, fill, 3.0), 1.5, blur, 0.0, p.glow.with(0.35 * 0.5));
    let g = cairo::LinearGradient::new(x, y, x + fill, y);
    stop(&g, 0.0, p.glow);
    stop(&g, 1.0, if segment == Segment::Leading { p.beam_core } else { p.glow });
    let _ = cr.set_source(&g);
    rounded(cr, x, y, fill, 3.0, 1.5);
    let _ = cr.fill();
}

// ---- The NEAR mark -------------------------------------------------------

/// The mark's place in a square `side` across with its top-left at
/// `origin`: the transform from the brand tile's coordinates.
fn mark_transform(cr: &cairo::Context, origin: Point, side: f64) {
    let [bx, by, bw, bh] = super::style::brand_tokens::logo::BRAND_MARK_VIEW_BOX;
    let fit = side.min(side * bw / bh);
    let scale = fit / bw;
    let left = origin.0 + (side - fit) / 2.0;
    let top = origin.1 + (side - bh * scale) / 2.0;
    cr.translate(left, top);
    cr.scale(scale, scale);
    cr.translate(-bx, -by);
}

/// `GlassBrandMarkDrawing` in a square `side` across at `origin`: filled
/// in the glow, inked as far as `drawn` along the marker line, the light
/// band at `shine`, and the pen tip at the stroke's front while it draws.
pub fn brand_mark(cr: &cairo::Context, origin: Point, side: f64, drawn: f64, shine: Option<f64>, tip: bool, p: &Palette) {
    if drawn <= 0.0 {
        return;
    }
    let Some(segments) = Artwork::brand_mark().cached_segments() else {
        return;
    };
    let _ = cr.save();
    mark_transform(cr, origin, side);
    super::glyph::trace_segments(cr, &segments);
    cr.set_fill_rule(cairo::FillRule::Winding);
    source(cr, p.glow);
    if drawn < 1.0 {
        // Cairo cannot clip to a stroke: clip to the mark and stroke the
        // marker's trimmed line in the fill, which inks the same pixels.
        let _ = cr.save();
        cr.clip();
        let line = stage::marker_trim(drawn);
        cr.move_to(line[0].0, line[0].1);
        for q in &line[1..] {
            cr.line_to(q.0, q.1);
        }
        cr.set_line_width(stage::MARKER_WIDTH);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        let _ = cr.stroke();
        let _ = cr.restore();
    } else {
        let _ = cr.fill();
    }
    if let Some(shine) = shine {
        let _ = cr.save();
        super::glyph::trace_segments(cr, &segments);
        cr.clip();
        let (lo, hi) = stage::SHINE_TRAVEL;
        let band = stage::shine_band(lo + (hi - lo) * shine);
        let g = cairo::LinearGradient::new(band.from.0, band.from.1, band.to.0, band.to.1);
        stop(&g, 0.0, Rgba(1.0, 1.0, 1.0, 0.0));
        stop(&g, 0.5, Rgba(1.0, 1.0, 1.0, 0.7));
        stop(&g, 1.0, Rgba(1.0, 1.0, 1.0, 0.0));
        let _ = cr.set_source(&g);
        cr.move_to(band.corners[0].0, band.corners[0].1);
        for c in &band.corners[1..] {
            cr.line_to(c.0, c.1);
        }
        cr.close_path();
        let _ = cr.fill();
        let _ = cr.restore();
    }
    if tip {
        if let Some((at, radius, fade)) = stage::pen_tip(drawn) {
            // The shadow's glow, then the tip in the hero ink.
            glow_disc(cr, at, radius * 2.6, p.glow.alpha(fade * 0.9));
            cr.arc(at.0, at.1, radius, 0.0, TAU);
            source(cr, p.hero_ink.alpha(fade));
            let _ = cr.fill();
        }
    }
    let _ = cr.restore();
}

/// How the N is drawn on a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hub {
    pub drawn: f64,
    pub shine: Option<f64>,
    pub tip: bool,
    pub glow: f64,
    pub charged: bool,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            drawn: 1.0,
            shine: None,
            tip: false,
            glow: 1.0,
            charged: false,
        }
    }
}

/// `FirstRunHub` in `frame`: the breathing glow behind the mark, then the
/// mark. `time` is the breathing clock.
pub fn hub(cr: &cairo::Context, frame: Frame, look: Hub, time: f64, reduce_motion: bool, alpha: f64, p: &Palette) {
    let side = frame.side;
    let disc = side * 220.0 / 132.0;
    let (scale, opacity) = stage::breathing(time, look.charged, reduce_motion);
    let peak = if p.dark { 0.35 } else { 0.2 };
    glow_disc(
        cr,
        frame.center,
        disc * 0.31 * scale,
        p.glow.with(peak * opacity * look.glow * alpha),
    );
    let origin = (frame.center.0 - side / 2.0, frame.center.1 - side / 2.0);
    if alpha < 1.0 {
        cr.push_group();
        brand_mark(cr, origin, side, look.drawn, look.shine, look.tip, p);
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(alpha);
    } else {
        brand_mark(cr, origin, side, look.drawn, look.shine, look.tip, p);
    }
}

// ---- Tools, orbs ---------------------------------------------------------

/// A tool's mark on its tile: logo artwork, or two letters, and the tint.
fn tool_mark(kind: &str, p: &Palette) -> (Option<Artwork>, Rgba, &'static str) {
    let logo = |name: &str| {
        Artwork::TOOLS
            .iter()
            .find_map(|(known, art)| (*known == name).then_some(*art))
    };
    match kind {
        "claude-code" => (logo("claude"), p.tool_claude, "CC"),
        "codex" => (logo("codex"), p.tool_codex, "Cx"),
        "gemini-cli" => (None, p.tool_antigravity, "Gm"),
        "opencode" => (logo("openCode"), p.tool_open_code, "OC"),
        _ => (None, p.text_primary, "Cl"),
    }
}

/// `FirstRunToolFace` in `frame`: the raised tile, its shadow, the logo in
/// its tint or its initials, and the glow round it while `lit`.
pub fn tool_face(cr: &cairo::Context, frame: Frame, kind: &str, lit: bool, alpha: f64, p: &Palette) {
    if alpha <= 0.0 || frame.side <= 0.0 {
        return;
    }
    let side = frame.side;
    let (x, y) = (frame.center.0 - side / 2.0, frame.center.1 - side / 2.0);
    let corner = side * 14.0 / 52.0;
    cr.push_group();
    if lit {
        soft_shadow(cr, (x, y, side, side), corner, side * 11.0 / 52.0, 0.0, p.glow.with(0.35 * 0.6));
    }
    soft_shadow(
        cr,
        (x, y, side, side),
        corner,
        side * 10.0 / 52.0,
        side * 8.0 / 52.0,
        p.tile_shadow.alpha(0.5),
    );
    rounded(cr, x, y, side, side, corner);
    source(cr, p.tile);
    let _ = cr.fill();
    let (logo, tint, initials) = tool_mark(kind, p);
    source(cr, tint);
    match logo {
        Some(art) => art.fill(cr, (x + side / 4.0, y + side / 4.0), side / 2.0),
        None => {
            cr.select_font_face("sans-serif", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size((side * 16.0 / 52.0).max(6.0));
            if let Ok(e) = cr.text_extents(initials) {
                cr.move_to(
                    frame.center.0 - e.x_bearing() - e.width() / 2.0,
                    frame.center.1 - e.y_bearing() - e.height() / 2.0,
                );
                let _ = cr.show_text(initials);
            }
            cr.new_path();
        }
    }
    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(alpha);
}

/// `FirstRunOrb`: white at its heart fading through the beam's blue.
pub fn orb(cr: &cairo::Context, center: Point, diameter: f64, alpha: f64, p: &Palette) {
    if alpha <= 0.0 {
        return;
    }
    let r = diameter / 2.0;
    let g = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, r);
    stop(&g, 0.0, p.hero_ink.alpha(alpha));
    stop(&g, 0.22, p.beam_core.alpha(alpha));
    stop(&g, 0.46, p.beam.with(0.65 * alpha));
    stop(&g, 0.72, p.glow.with(0.0));
    stop(&g, 1.0, p.glow.with(0.0));
    let _ = cr.set_source(&g);
    cr.arc(center.0, center.1, r, 0.0, TAU);
    let _ = cr.fill();
}

fn dashed_circle(cr: &cairo::Context, center: Point, radius: f64, width: f64, dash: [f64; 2], turn: f64, c: Rgba) {
    let _ = cr.save();
    cr.translate(center.0, center.1);
    cr.rotate(turn.to_radians());
    cr.set_line_width(width);
    cr.set_dash(&dash, 0.0);
    source(cr, c);
    cr.arc(0.0, 0.0, radius, -PI / 2.0, 1.5 * PI);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let _ = cr.restore();
}

// ---- Each step's art, apart from its actors ------------------------------

/// What a step's art needs beyond the actors.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// The ring's rows in ring order, and which are watched.
    pub tools: stage::ToolsSeen,
    /// Seconds since the Tools scan started, while it runs; None once over.
    pub scan: Option<f64>,
    /// Seconds since the step opened (Traces' count, Private AI's flow).
    pub opened: f64,
    /// The Uses path, and seconds since it was chosen.
    pub sharing: Sharing,
    pub sharing_since: f64,
    /// Seconds since a sign-in on Join, while its ring of light runs.
    pub signed_in: Option<f64>,
    /// The breathing clock, in seconds.
    pub time: f64,
    pub reduce_motion: bool,
}

/// The Tools ring's art behind the actors: the dashed ring turning slowly,
/// the sweep, and each watched tile's beam into the N.
pub fn tools_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    let center = (stage::CANVAS.0 / 2.0, stage::ring::CENTER_Y);
    let settled = scene.scan.is_none_or(stage::scan_done);
    let opacity = ((scene.scan.unwrap_or(10.0) - 0.3) / 0.7).clamp(0.0, 1.0);
    let radius = if settled {
        stage::ring::TIGHT_RADIUS
    } else {
        stage::ring::LOOSE_RADIUS
    };
    dashed_circle(
        cr,
        center,
        radius,
        1.5 * radius / stage::ring::LOOSE_RADIUS,
        [3.0 * radius / stage::ring::LOOSE_RADIUS, 7.0 * radius / stage::ring::LOOSE_RADIUS],
        stage::ring::turn(scene.time, scene.reduce_motion),
        p.rule.alpha(opacity),
    );
    if let Some(rotation) = scene.scan.and_then(stage::sweep_rotation) {
        sweep(cr, center, rotation, scene.scan.unwrap_or(0.0), p);
    }
    if settled {
        let exists: Vec<bool> = scene.tools.rows.iter().map(|(_, e)| *e).collect();
        for (index, (kind, found)) in scene.tools.rows.iter().enumerate() {
            if !found || !scene.tools.watched.iter().any(|w| w == kind) {
                continue;
            }
            let angle = stage::ring::placement(index, &exists, None).angle;
            beam(
                cr,
                stage::point(center, 90.0, angle),
                stage::point(center, 16.0, angle),
                scene.time,
                scene.reduce_motion,
                p,
            );
        }
    }
}

/// The radar sweep: a band with a long trail behind it, clear near the N.
/// Cairo has no conic gradient, so the trail is drawn as thin wedges.
fn sweep(cr: &cairo::Context, center: Point, rotation: f64, elapsed: f64, p: &Palette) {
    let turn_end = stage::SCAN_LEAD + stage::SCAN_TURN;
    let fade = 1f64
        .min(((elapsed - stage::SCAN_LEAD) / 0.4).max(0.0))
        .min(((turn_end - elapsed) / 0.4).max(0.0));
    if fade <= 0.0 {
        return;
    }
    let stops: [(f64, Rgba); 8] = [
        (0.0, p.glow.with(0.0)),
        (140.0 / 360.0, p.glow.with(0.0)),
        (210.0 / 360.0, p.glow.with(0.03)),
        (280.0 / 360.0, p.glow.with(0.11)),
        (330.0 / 360.0, p.glow.with(0.24)),
        (352.0 / 360.0, p.beam.with(0.4)),
        (358.0 / 360.0, p.beam.with(0.5)),
        (1.0, p.glow.with(0.0)),
    ];
    let colour = |at: f64| {
        for w in stops.windows(2) {
            if at <= w[1].0 {
                let t = (at - w[0].0) / (w[1].0 - w[0].0);
                let (a, b) = (w[0].1, w[1].1);
                return Rgba(
                    a.0 + (b.0 - a.0) * t,
                    a.1 + (b.1 - a.1) * t,
                    a.2 + (b.2 - a.2) * t,
                    a.3 + (b.3 - a.3) * t,
                );
            }
        }
        stops[7].1
    };
    cr.push_group();
    let wedges = 180;
    for i in 0..wedges {
        let at = (i as f64 + 0.5) / wedges as f64;
        // From the top, clockwise, turned with the sweep.
        let a0 = (-90.0 + rotation + 360.0 * i as f64 / wedges as f64).to_radians();
        let a1 = (-90.0 + rotation + 360.0 * (i + 1) as f64 / wedges as f64).to_radians() + 0.004;
        cr.move_to(center.0, center.1);
        cr.arc(center.0, center.1, 150.0, a0, a1);
        cr.close_path();
        source(cr, colour(at));
        let _ = cr.fill();
    }
    let _ = cr.pop_group_to_source();
    let mask = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, 150.0);
    mask.add_color_stop_rgba(36.0 / 150.0, 0.0, 0.0, 0.0, 0.0);
    mask.add_color_stop_rgba(64.0 / 150.0, 0.0, 0.0, 0.0, fade);
    mask.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, fade);
    let _ = cr.mask(&mask);
}

/// One watched tile's beam into the N (`ToolsBeam`): a wide halo, a body, a
/// core and soft pulses flowing in, each fading toward the N.
fn beam(cr: &cairo::Context, from: Point, to: Point, time: f64, reduce_motion: bool, p: &Palette) {
    let breath = if reduce_motion {
        0.0
    } else {
        0.5 - 0.5 * (TAU * time / 2.2).cos()
    };
    let flow = if reduce_motion {
        0.0
    } else {
        (time / 1.1).rem_euclid(1.0) * 26.1
    };
    let body = |opacity: f64| {
        let g = cairo::LinearGradient::new(from.0, from.1, to.0, to.1);
        stop(&g, 0.0, p.beam.with(0.95 * opacity));
        stop(&g, 0.55, p.beam.with(0.4 * opacity));
        stop(&g, 1.0, p.beam.with(0.0));
        g
    };
    let line = |width: f64, g: &cairo::LinearGradient| {
        cr.set_line_width(width);
        cr.set_line_cap(cairo::LineCap::Round);
        let _ = cr.set_source(g);
        cr.move_to(from.0, from.1);
        cr.line_to(to.0, to.1);
        let _ = cr.stroke();
    };
    // The Mac blurs each layer; a blurred stroke is about its width plus
    // twice the blur, at a lower peak.
    line(24.0 + 6.0 * breath + 14.0, &body((0.34 + 0.16 * breath) * 0.55));
    line(9.0 + 2.0 * breath + 5.0, &body((0.55 + 0.17 * breath) * 0.7));
    line(2.6 + 1.8, &body(0.8));
    let pulses = cairo::LinearGradient::new(from.0, from.1, to.0, to.1);
    stop(&pulses, 0.0, p.beam_core.with(0.95));
    stop(&pulses, 0.6, p.beam_core.with(0.45 * 0.95));
    stop(&pulses, 1.0, p.beam_core.with(0.0));
    cr.set_dash(&[0.1, 26.0], flow);
    line(8.0, &pulses);
    cr.set_dash(&[], 0.0);
    glow_disc(cr, from, 13.0 + 7.0, p.beam.with(0.4 * 0.6));
}

/// The flash round tile `frame` as the sweep reaches it.
pub fn tile_flash(cr: &cairo::Context, frame: Frame, flash: f64, p: &Palette) {
    let side = frame.side + 14.0;
    let scale = 0.88 + 0.28 * flash;
    let opacity = if flash < 0.4 {
        flash / 0.4
    } else {
        (1.0 - flash) / 0.6
    };
    let s = side * scale;
    rounded(cr, frame.center.0 - s / 2.0 + 1.0, frame.center.1 - s / 2.0 + 1.0, s - 2.0, s - 2.0, 19.0 * scale);
    cr.set_line_width(2.0);
    source(cr, p.glow.alpha(opacity));
    let _ = cr.stroke();
}

/// Private AI's art behind the actors: each tool's dashed route into the N
/// and the N's line on toward NEAR AI's pill.
pub fn private_ai_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    use stage::private_ai as pa;
    let flow = if scene.reduce_motion {
        0.0
    } else {
        (scene.time / 1.1).rem_euclid(1.0)
    };
    cr.set_line_width(2.0);
    cr.set_dash(&[5.0, 6.0], -22.0 * flow);
    source(cr, p.glow);
    let count = scene.tools.connectable.len();
    for index in 0..count {
        let y = pa::row_y(index, count);
        cr.move_to(pa::ROUTE_START, y);
        cr.curve_to(330.0, y, 350.0, pa::HUB.1, pa::HUB.0 - 36.0, pa::HUB.1);
        let _ = cr.stroke();
    }
    cr.move_to(pa::HUB.0 + 36.0, pa::HUB.1);
    cr.line_to(pa::DESTINATION_X - pa::PILL_WIDTH / 2.0, pa::HUB.1);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

/// Traces' art: each watched tool's stream into the ring, the orbs down
/// it, the ring's well and the count's arc rising round it.
pub fn traces_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    use stage::traces as tr;
    let watched = scene.tools.watched_in_ring_order();
    let center = tr::CENTER;
    let elapsed = scene.opened;
    for index in 0..watched.len() {
        let y = tr::row_y(index, watched.len(), center.1);
        let curve = tr::stream((tr::STREAM_X, y), (center.0 - 114.0, center.1));
        cr.set_line_width(1.5);
        cr.set_dash(&[2.0, 5.0], 0.0);
        source(cr, p.rule);
        cr.move_to(curve[0].0, curve[0].1);
        cr.curve_to(curve[1].0, curve[1].1, curve[2].0, curve[2].1, curve[3].0, curve[3].1);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        if tr::flowing(elapsed, scene.reduce_motion) {
            for o in 0..3 {
                let into = (elapsed - 0.5 - o as f64 * 0.46 - index as f64 * 0.15) / 1.38;
                if into >= 0.0 {
                    let t = into % 1.0;
                    let eased = stage::motion_curve(t);
                    let alpha = if t < 0.12 {
                        t / 0.12
                    } else if t > 0.88 {
                        (1.0 - t) / 0.12
                    } else {
                        1.0
                    };
                    orb(cr, tr::bezier(curve, eased), 14.0, alpha, p);
                }
            }
        }
    }
    let r = tr::RING / 2.0;
    cr.set_line_width(8.0);
    source(cr, p.well);
    cr.arc(center.0, center.1, r, 0.0, TAU);
    let _ = cr.stroke();
    let counted = tr::counted(elapsed, scene.reduce_motion);
    if counted > 0.0 {
        let start = -PI / 2.0;
        let end = start + TAU * counted;
        cr.set_line_cap(cairo::LineCap::Round);
        for (w, a) in [(14.0, 0.12), (11.0, 0.12)] {
            cr.set_line_width(w);
            source(cr, p.glow.with(a));
            cr.arc(center.0, center.1, r, start, end);
            let _ = cr.stroke();
        }
        cr.set_line_width(8.0);
        source(cr, p.glow);
        cr.arc(center.0, center.1, r, start, end);
        let _ = cr.stroke();
        cr.set_line_cap(cairo::LineCap::Butt);
    }
}

/// The Uses flow's art: the line, This Mac's box, the review gate, the
/// commons' pulse and ring of arrived traces, and the orb on its way.
/// The thumbs-up inside the gate is an SF Symbol on the Mac and is not
/// drawn here.
pub fn uses_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    use stage::uses as us;
    let y = us::LINE_Y;
    cr.set_line_width(2.0);
    cr.set_dash(&[4.0, 6.0], 0.0);
    source(cr, p.rule);
    cr.move_to(us::LINE_START, y);
    cr.line_to(us::LINE_END, y);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    // This Mac's box.
    rounded(cr, us::MAC_X - 56.0, y - 39.0, 112.0, 78.0, 16.0);
    source(cr, p.card);
    let _ = cr.fill();
    rounded(cr, us::MAC_X - 22.0, y + 39.0 - 13.0, 44.0, 3.0, 1.5);
    source(cr, p.rule);
    let _ = cr.fill();
    let t = if scene.reduce_motion {
        None
    } else {
        us::looped(scene.sharing, scene.sharing_since)
    };
    // The gate.
    let look = t.map_or(us::GATE_REST, |t| us::gate(scene.sharing, t));
    let gate = (us::GATE_X, y);
    soft_shadow(cr, (gate.0 - 29.0, gate.1 - 29.0, 58.0, 58.0), 29.0, 10.0, 8.0, p.tile_shadow.alpha(0.5));
    cr.arc(gate.0, gate.1, 29.0, 0.0, TAU);
    source(cr, p.card_solid);
    let _ = cr.fill();
    if look.fill > 0.0 {
        let c = (gate.0, gate.1 - 29.0 + 58.0 * 0.38);
        let g = cairo::RadialGradient::new(c.0, c.1, 0.0, c.0, c.1, 30.0);
        stop(&g, 0.0, p.approve.alpha(look.fill));
        stop(&g, 1.0, p.approve_deep.alpha(look.fill));
        let _ = cr.set_source(&g);
        cr.arc(gate.0, gate.1, 29.0, 0.0, TAU);
        let _ = cr.fill();
    }
    // The commons.
    let commons = (us::COMMONS_X, y);
    if let Some(t) = t {
        let (scale, opacity) = us::pulse(scene.sharing, t);
        if opacity > 0.0 {
            let r = 60.0 * scale;
            let g = cairo::RadialGradient::new(commons.0, commons.1, 0.0, commons.0, commons.1, r);
            stop(&g, 0.3, p.beam.with(0.0));
            stop(&g, 0.46, p.beam.with(0.6 * opacity));
            stop(&g, 0.64, p.beam.with(0.0));
            let _ = cr.set_source(&g);
            cr.arc(commons.0, commons.1, r, 0.0, TAU);
            let _ = cr.fill();
        }
    }
    let turn = if scene.reduce_motion {
        0.0
    } else {
        (scene.sharing_since / 40.0).rem_euclid(1.0) * 360.0
    };
    dashed_circle(cr, commons, 45.5, 1.0, [3.0, 4.0], turn, p.rule);
    let filled = if scene.reduce_motion {
        0
    } else {
        us::satellites(scene.sharing, scene.sharing_since)
    };
    for place in 0..filled.min(5) {
        let at = stage::point(commons, 46.0, stage::ring_angle(place, 5) + turn);
        orb(cr, at, 14.0, 1.0, p);
    }
    if let Some(t) = t {
        let (along, opacity) = us::orb(scene.sharing, t);
        orb(
            cr,
            (us::LINE_START + (us::LINE_END - us::LINE_START) * along, y),
            18.0,
            opacity,
            p,
        );
    }
}

/// Join's orbit behind the actors: the dashed ring turning slowly, and a
/// sign-in's ring of light from the N.
pub fn join_back(cr: &cairo::Context, scene: &Scene, p: &Palette) {
    let center = (stage::CANVAS.0 / 2.0, stage::join::BOX / 2.0);
    dashed_circle(
        cr,
        center,
        stage::join::RING / 2.0 - 0.5,
        1.0,
        [3.0, 4.0],
        stage::ring::turn(scene.time, scene.reduce_motion),
        p.rule,
    );
    if scene.reduce_motion {
        return;
    }
    if let Some((scale, opacity)) = scene.signed_in.and_then(stage::join::pulse) {
        let r = 46.0 * scale;
        let g = cairo::RadialGradient::new(center.0, center.1, 0.0, center.0, center.1, r);
        stop(&g, 0.26, p.beam.with(0.0));
        stop(&g, 0.44, p.beam.with(0.6 * opacity));
        stop(&g, 0.64, p.beam.with(0.0));
        let _ = cr.set_source(&g);
        cr.arc(center.0, center.1, r, 0.0, TAU);
        let _ = cr.fill();
    }
}

/// A step's art behind its actors, in the step graphic's canvas.
pub fn step_back(cr: &cairo::Context, step: crate::first_run::Step, scene: &Scene, p: &Palette) {
    use crate::first_run::Step;
    match step {
        Step::Tools => tools_back(cr, scene, p),
        Step::PrivateAi => private_ai_back(cr, scene, p),
        Step::Traces => traces_back(cr, scene, p),
        Step::Uses => uses_back(cr, scene, p),
        Step::Join => join_back(cr, scene, p),
        Step::Welcome | Step::Terms => {}
    }
}

/// Where tile `kind` is on `step` this moment, and how it is drawn: the
/// Tools ring's scan and bob, or the settled frame.
pub fn live_tile(
    step: crate::first_run::Step,
    kind: &str,
    scene: &Scene,
) -> Option<(Frame, f64, Option<f64>)> {
    use crate::first_run::Step;
    if step != Step::Tools {
        return stage::tile_frame(step, kind, &scene.tools).map(|f| (f, 1.0, None));
    }
    let exists: Vec<bool> = scene.tools.rows.iter().map(|(_, e)| *e).collect();
    let index = scene.tools.rows.iter().position(|(k, _)| k == kind)?;
    let place = stage::ring::placement(index, &exists, scene.scan);
    if place.opacity <= 0.0 {
        return None;
    }
    let bob = if scene.reduce_motion {
        0.0
    } else {
        stage::ring::bob(kind, scene.time)
    };
    let center = stage::point((stage::CANVAS.0 / 2.0, stage::ring::CENTER_Y), place.radius, place.angle);
    Some((
        Frame {
            center: (center.0, center.1 + bob * place.scale),
            side: stage::ring::TILE * place.scale,
        },
        place.opacity,
        stage::ring::flash(index, exists.len(), scene.scan),
    ))
}

/// How the N looks on `step`: drawn and shining on Welcome, charged on
/// Tools while a tool is watched.
pub fn hub_look(step: crate::first_run::Step, scene: &Scene, welcome_elapsed: f64) -> Hub {
    use crate::first_run::Step;
    match step {
        Step::Welcome => Hub {
            drawn: stage::welcome::drawn(welcome_elapsed, scene.reduce_motion),
            shine: stage::welcome::shine(welcome_elapsed, scene.reduce_motion),
            tip: true,
            glow: stage::welcome::glow(welcome_elapsed, scene.reduce_motion),
            charged: false,
        },
        Step::Tools => Hub {
            charged: !scene.tools.watched.is_empty(),
            ..Hub::default()
        },
        _ => Hub::default(),
    }
}

/// A whole step graphic, settled or mid-scan, its actors where the step
/// puts them: the comparison renders draw this.
pub fn step_graphic(cr: &cairo::Context, step: crate::first_run::Step, scene: &Scene, welcome_elapsed: f64, p: &Palette) {
    step_back(cr, step, scene, p);
    let hub_frame = stage::hub_frame(step);
    hub(cr, hub_frame, hub_look(step, scene, welcome_elapsed), scene.time, scene.reduce_motion, 1.0, p);
    for kind in stage::RING_ORDER {
        if let Some((frame, alpha, flash)) = live_tile(step, kind, scene) {
            let lit = step == crate::first_run::Step::Tools && scene.tools.watched.iter().any(|w| w == kind);
            tool_face(cr, frame, kind, lit, alpha, p);
            if let Some(flash) = flash {
                tile_flash(cr, frame, flash, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::Step;

    fn surface(w: i32, h: i32) -> cairo::ImageSurface {
        cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap()
    }

    /// The premultiplied pixel at (x, y) as (r, g, b, a), 0 to 255.
    fn pixel(s: &mut cairo::ImageSurface, x: i32, y: i32) -> (u8, u8, u8, u8) {
        s.flush();
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let i = y as usize * stride + x as usize * 4;
        (data[i + 2], data[i + 1], data[i], data[i + 3])
    }

    #[test]
    fn the_palette_is_the_flat_themes() {
        let dark = Palette::scheme(true);
        assert_eq!(dark.scene_base, Rgba(7.0 / 255.0, 10.0 / 255.0, 26.0 / 255.0, 1.0));
        assert_eq!(dark.glow, Rgba(0.0, 145.0 / 255.0, 253.0 / 255.0, 1.0));
        assert_eq!(dark.rule.3, 0.18);
        let light = Palette::scheme(false);
        assert_eq!(light.glow, Rgba(0.0, 112.0 / 255.0, 198.0 / 255.0, 1.0));
    }

    #[test]
    fn the_mark_inks_along_its_line() {
        let p = Palette::scheme(true);
        // A half-drawn mark has its lower-left inked and its upper-right
        // hook still bare; the whole mark has both.
        let ink = |drawn: f64| {
            let mut s = surface(400, 400);
            {
                let cr = cairo::Context::new(&s).unwrap();
                cr.scale(1.0, 1.0);
                // The tile's own coordinates: a 400-wide square at 0.
                brand_mark(&cr, (0.0, 0.0), 400.0, drawn, None, false, &p);
            }
            let lower_left = pixel(&mut s, 101, 290).3;
            let upper_right = pixel(&mut s, 280, 112).3;
            (lower_left, upper_right)
        };
        let (a, b) = ink(0.3);
        assert!(a > 200 && b == 0, "{a} {b}");
        let (a, b) = ink(1.0);
        assert!(a > 200 && b > 200, "{a} {b}");
        assert_eq!(ink(0.0), (0, 0));
    }

    #[test]
    fn the_backdrop_glows_from_the_top_left() {
        let p = Palette::scheme(true);
        let mut s = surface(860, 760);
        {
            let cr = cairo::Context::new(&s).unwrap();
            backdrop(&cr, 860.0, 760.0, &p, 1.0);
        }
        let corner = pixel(&mut s, 180, 150);
        let far = pixel(&mut s, 850, 750);
        assert!(corner.2 > far.2 + 60, "{corner:?} {far:?}");
        assert_eq!(far.3, 255);
    }

    #[test]
    fn every_step_graphic_draws_its_n() {
        let p = Palette::scheme(true);
        let scene = Scene {
            tools: stage::ToolsSeen {
                rows: vec![("claude-code".into(), true), ("codex".into(), true)],
                watched: vec!["codex".into()],
                connectable: vec!["codex".into()],
            },
            ..Scene::default()
        };
        for step in Step::ALL {
            let h = stage::graphic_height(step).ceil() as i32;
            let mut s = surface(860, h);
            {
                let cr = cairo::Context::new(&s).unwrap();
                step_graphic(&cr, step, &scene, 5.0, &p);
            }
            let f = stage::hub_frame(step);
            // Inside the left stem of the N.
            let x = (f.center.0 - f.side * 0.4) as i32;
            let y = f.center.1 as i32;
            assert!(pixel(&mut s, x, y).3 > 200, "{step:?}");
        }
    }

    /// Renders the stage's pieces as the Mac probe renders them (the same
    /// sizes, the same clock, at 2x), each step's graphic, and each step's
    /// stage at the first-run window's default size, as raw premultiplied
    /// BGRA into `TC_STAGE_RENDER_DIR`.
    #[test]
    #[ignore = "writes comparison renders; set TC_STAGE_RENDER_DIR"]
    fn render_the_stage_for_comparison() {
        let Ok(dir) = std::env::var("TC_STAGE_RENDER_DIR") else {
            return;
        };
        let write = |name: &str, w: i32, h: i32, draw: &dyn Fn(&cairo::Context)| {
            let mut s = surface(w * 2, h * 2);
            {
                let cr = cairo::Context::new(&s).unwrap();
                cr.scale(2.0, 2.0);
                draw(&cr);
            }
            s.flush();
            let path = format!("{dir}/{name}-{}x{}.bgra", w * 2, h * 2);
            std::fs::write(path, &*s.data().unwrap()).unwrap();
        };
        let time = 1000.0;
        let rows: Vec<(String, bool)> = [
            ("claude-code", true),
            ("gemini-cli", false),
            ("codex", true),
            ("cline", true),
            ("opencode", false),
        ]
        .iter()
        .map(|(k, e)| ((*k).to_owned(), *e))
        .collect();
        for dark in [true, false] {
            let p = Palette::scheme(dark);
            let scheme = if dark { "dark" } else { "light" };
            write(&format!("backdrop-{scheme}"), 860, 760, &|cr| {
                backdrop(cr, 860.0, 760.0, &p, stage::backdrop_phase(time, false))
            });
            for e in [0.5, 0.8, 1.9, 3.0] {
                let scene = Scene {
                    time,
                    ..Scene::default()
                };
                write(&format!("welcome-hub-{e}-{scheme}"), 240, 240, &|cr| {
                    let frame = Frame {
                        center: (120.0, 120.0),
                        side: 132.0,
                    };
                    hub(cr, frame, hub_look(Step::Welcome, &scene, e), time, false, 1.0, &p);
                });
            }
            for (name, charged) in [("hub", false), ("hub-charged", true)] {
                write(&format!("{name}-{scheme}"), 120, 120, &|cr| {
                    let look = Hub {
                        charged,
                        ..Hub::default()
                    };
                    let frame = Frame {
                        center: (60.0, 60.0),
                        side: 64.0,
                    };
                    hub(cr, frame, look, time, false, 1.0, &p);
                });
            }
            for kind in stage::RING_ORDER {
                let mac = match kind {
                    "claude-code" => "claudeCode",
                    "gemini-cli" => "geminiCli",
                    other => other,
                };
                write(&format!("tile-{mac}-{scheme}"), 80, 80, &|cr| {
                    let frame = Frame {
                        center: (40.0, 40.0),
                        side: 52.0,
                    };
                    tool_face(cr, frame, kind, kind == "codex", 1.0, &p);
                });
            }
            write(&format!("orb-{scheme}"), 18, 18, &|cr| orb(cr, (9.0, 9.0), 18.0, 1.0, &p));
            for e in [0.5, 1.6, 2.4, 4.5] {
                let scene = Scene {
                    tools: stage::ToolsSeen {
                        rows: rows.clone(),
                        watched: vec!["codex".into()],
                        connectable: vec![],
                    },
                    scan: (e < 4.0).then_some(e),
                    time,
                    ..Scene::default()
                };
                write(&format!("tools-ring-{e}-{scheme}"), 860, 300, &|cr| {
                    step_graphic(cr, Step::Tools, &scene, 0.0, &p)
                });
            }
            let tools = stage::ToolsSeen {
                rows: rows.clone(),
                watched: vec!["claude-code".into(), "codex".into(), "cline".into()],
                connectable: vec!["claude-code".into(), "codex".into()],
            };
            for step in Step::ALL {
                for at in [0.6, 2.0, 6.0] {
                    let scene = Scene {
                        tools: tools.clone(),
                        opened: at,
                        sharing_since: at + 4.0,
                        time: time + at,
                        ..Scene::default()
                    };
                    let h = stage::graphic_height(step);
                    // The step graphic in its canvas strip.
                    write(&format!("graphic-{step:?}-{at}-{scheme}"), 860, h.ceil() as i32, &|cr| {
                        step_graphic(cr, step, &scene, at, &p)
                    });
                    // The stage at the window's default size: the scene,
                    // then the graphic scaled to fit, under the thread.
                    write(&format!("stage-{step:?}-{at}-{scheme}"), 680, 720, &|cr| {
                        backdrop(cr, 680.0, 720.0, &p, stage::backdrop_phase(time + at, false));
                        let scale = stage::fit_scale(680.0, 720.0);
                        let _ = cr.save();
                        cr.translate(0.0, 92.0);
                        cr.scale(scale, scale);
                        step_graphic(cr, step, &scene, at, &p);
                        let _ = cr.restore();
                    });
                }
            }
        }
    }
}
