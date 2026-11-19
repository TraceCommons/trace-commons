//! A hero card on Home: the Mac's `GlassHeroCard` as GTK widgets.
//!
//! The ground and the pixel graphic are one drawing area under the words
//! (`hero_art::paint_card`); the words, the caller's content and the white
//! pill are labels over it; an optional graphic made of widgets (the
//! Insights hero's bars) sits between them, placed past the card's
//! trailing and bottom edges, and the card clips it. The whole card is one
//! button that names the action, read once; the pill draws the action and
//! is not a second control. A closable card's X is a control of its own
//! over the card's top trailing corner.
//!
//! Under the pointer heroInk at 6% lies over the ground, under the graphic.
//!
//! The pixel graphic moves at `hero_art::field::FRAMES_PER_SECOND` on the
//! drawing area's frame clock, which runs only while the card is mapped;
//! with `gtk-enable-animations` off it is drawn still at its first frame
//! and nothing ticks. Every word on it is the caller's, from shared copy.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::hero_art::{self, Ground, card, glyph::Mask};
use super::style::brand_tokens::glass::space;

/// The card's state the drawing reads.
struct Paint {
    ground: Ground,
    hover: Cell<bool>,
    /// The hover wash, 0 to 1, fading over `HOVER_FADE`.
    wash: super::glass_motion::Tween,
    mask: RefCell<Option<Mask>>,
    /// The clock's first frame, so the graphic's time starts at 0.
    epoch: Cell<Option<i64>>,
    /// When the timer last fired, in microseconds; 0 before it has.
    last: Cell<i64>,
    time: Cell<f64>,
}

pub struct HeroCard {
    pub root: gtk::Overlay,
    /// The card itself: one button.
    pub button: gtk::Button,
    /// The caller's content, between the line and the pill.
    pub content: gtk::Box,
    pub title: gtk::Label,
    pub subtitle: gtk::Label,
    pub pill: Option<gtk::Label>,
    pub close: Option<gtk::Button>,
    canvas: gtk::DrawingArea,
    layers: gtk::Overlay,
    paint: Rc<Paint>,
    /// Starts or stops the timer for what is true now.
    restart: RefCell<Option<Rc<dyn Fn()>>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// The graphic made of widgets, if any, and the words over it.
    graphic: Rc<RefCell<Option<gtk::Widget>>>,
    words: glib::WeakRef<gtk::Box>,
}

/// How long the hover wash takes: `GlassMotion.fast`.
pub const HOVER_FADE: f64 = super::style::brand_tokens::motion::FAST;

/// The pointer entering or leaving: the wash fades on the token curve.
fn fade_hover(paint: &Paint, canvas: &gtk::DrawingArea, hover: bool) {
    paint.hover.set(hover);
    let weak = canvas.downgrade();
    paint.wash.to(
        canvas,
        if hover { 1.0 } else { 0.0 },
        HOVER_FADE,
        super::glass_motion::curve,
        move |_| {
            if let Some(canvas) = weak.upgrade() {
                canvas.queue_draw();
            }
        },
    );
}

/// Whether this desktop wants motion.
pub fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations())
}

impl HeroCard {
    pub fn new(
        ground: Ground,
        title: &str,
        subtitle: Option<&str>,
        action_title: Option<&str>,
        close_label: Option<&str>,
    ) -> Self {
        let paint = Rc::new(Paint {
            ground,
            hover: Cell::new(false),
            wash: super::glass_motion::Tween::new(0.0),
            mask: RefCell::new(None),
            epoch: Cell::new(None),
            last: Cell::new(0),
            time: Cell::new(0.0),
        });

        let canvas = gtk::DrawingArea::builder()
            .hexpand(true)
            .content_height(card::MIN_HEIGHT as i32)
            .can_target(false)
            .build();
        {
            let paint = Rc::clone(&paint);
            canvas.set_draw_func(move |area, cr, width, height| {
                let dark = adw::StyleManager::default().is_dark();
                let _ = area;
                hero_art::paint_card_washed(
                    cr,
                    (width as f64, height as f64),
                    paint.ground,
                    paint.wash.value(),
                    paint.mask.borrow().as_ref(),
                    dark,
                    paint.time.get(),
                );
            });
        }

        let words = gtk::Box::new(gtk::Orientation::Vertical, space::S3);
        let pad = card::PADDING;
        words.set_margin_top(pad);
        words.set_margin_bottom(pad);
        words.set_margin_start(pad);
        words.set_margin_end(pad + card::GRAPHIC_RESERVE);
        words.set_valign(gtk::Align::Start);
        let head = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
        let title_label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .wrap(true)
            .build();
        title_label.add_css_class("tc-type-display");
        title_label.add_css_class("tc-hero-title");
        title_label.set_accessible_role(gtk::AccessibleRole::Heading);
        head.append(&title_label);
        let subtitle_label = gtk::Label::builder()
            .label(subtitle.unwrap_or_default())
            .xalign(0.0)
            .wrap(true)
            .visible(subtitle.is_some_and(|s| !s.is_empty()))
            .build();
        subtitle_label.add_css_class("tc-type-label");
        subtitle_label.add_css_class("tc-hero-subtitle");
        head.append(&subtitle_label);
        words.append(&head);
        let content = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
        words.append(&content);
        let pill = action_title.filter(|a| !a.is_empty()).map(|action| {
            let pill = gtk::Label::builder()
                .label(action)
                .halign(gtk::Align::Start)
                .build();
            pill.add_css_class("tc-type-label");
            pill.add_css_class("tc-hero-pill");
            pill.set_margin_top(card::PILL_GAP);
            words.append(&pill);
            pill
        });

        let layers = gtk::Overlay::new();
        layers.set_child(Some(&canvas));
        layers.add_overlay(&words);
        layers.set_measure_overlay(&words, true);
        layers.set_overflow(gtk::Overflow::Hidden);
        layers.add_css_class("tc-hero-layers");
        let graphic: Rc<RefCell<Option<gtk::Widget>>> = Rc::new(RefCell::new(None));
        {
            let graphic = Rc::clone(&graphic);
            layers.connect_get_child_position(move |layers, child| {
                if graphic.borrow().as_ref() != Some(child) {
                    return None;
                }
                let (_, natural, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let (_, height, _, _) = child.measure(gtk::Orientation::Vertical, natural);
                let (x, y) = place_bars(layers.width(), layers.height(), natural, height);
                Some(gtk::gdk::Rectangle::new(x, y, natural, height))
            });
        }

        let button = gtk::Button::builder().child(&layers).hexpand(true).build();
        button.add_css_class("tc-hero");
        // Read once: the title, the line and the action.
        let name = [Some(title), subtitle, action_title]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(". ");
        button.update_property(&[gtk::accessible::Property::Label(&name)]);

        let motion = gtk::EventControllerMotion::new();
        {
            let paint = Rc::clone(&paint);
            let canvas = canvas.clone();
            motion.connect_enter(move |_, _, _| fade_hover(&paint, &canvas, true));
        }
        {
            let paint = Rc::clone(&paint);
            let canvas = canvas.clone();
            motion.connect_leave(move |_| fade_hover(&paint, &canvas, false));
        }
        button.add_controller(motion);

        let root = gtk::Overlay::new();
        root.set_child(Some(&button));
        let close = close_label.map(|label| {
            let image = gtk::Image::from_icon_name("window-close-symbolic");
            image.set_pixel_size(card::CLOSE_GLYPH);
            let close = gtk::Button::builder()
                .child(&image)
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .tooltip_text(label)
                .build();
            close.add_css_class("tc-hero-close");
            close.update_property(&[gtk::accessible::Property::Label(label)]);
            root.add_overlay(&close);
            close
        });

        let card = Self {
            root,
            button,
            content,
            title: title_label,
            subtitle: subtitle_label,
            pill,
            close,
            canvas,
            layers,
            paint,
            restart: RefCell::new(None),
            timer: Rc::new(RefCell::new(None)),
            graphic,
            words: words.downgrade(),
        };
        card.run_clock();
        card
    }

    /// The pixel graphic for `symbol`; none draws no glyph.
    pub fn set_symbol(&self, symbol: Option<&str>) {
        *self.paint.mask.borrow_mut() = symbol.and_then(hero_art::glyph::hero_mask);
        self.canvas.queue_draw();
        if let Some(restart) = self.restart.borrow().as_ref() {
            restart();
        }
    }

    /// A graphic made of widgets, between the ground and the words, placed
    /// by `place_bars` past the card's trailing and bottom edges; `None`
    /// takes it away.
    pub fn set_graphic(&self, graphic: Option<&impl IsA<gtk::Widget>>) {
        if let Some(old) = self.graphic.borrow_mut().take() {
            self.layers.remove_overlay(&old);
        }
        let Some(graphic) = graphic else { return };
        let graphic = graphic.as_ref().clone();
        graphic.set_can_target(false);
        self.layers.add_overlay(&graphic);
        // Under the words, which were added first.
        if let Some(words) = self.words.upgrade() {
            words.insert_after(&self.layers, Some(&graphic));
        }
        *self.graphic.borrow_mut() = Some(graphic);
    }

    pub fn graphic(&self) -> Option<gtk::Widget> {
        self.graphic.borrow().clone()
    }

    /// Hover as the pointer leaves it, at once: for a test or a capture.
    pub fn set_hover(&self, hover: bool) {
        self.paint.hover.set(hover);
        let canvas = self.canvas.clone();
        self.paint
            .wash
            .set(if hover { 1.0 } else { 0.0 }, move |_| canvas.queue_draw());
    }

    /// Hover as the pointer gives it: the wash fades.
    pub fn hover_to(&self, hover: bool) {
        fade_hover(&self.paint, &self.canvas, hover);
    }

    /// How far the hover wash is in, 0 to 1.
    pub fn wash(&self) -> f64 {
        self.paint.wash.value()
    }

    pub fn is_hovered(&self) -> bool {
        self.paint.hover.get()
    }

    /// The graphic's time, in seconds since the clock's first frame.
    pub fn time(&self) -> f64 {
        self.paint.time.get()
    }

    /// Ticks at the pixel graphic's rate (a timer, so the frame clock is
    /// not kept running at the display's rate for a 10 fps graphic), only
    /// while the card is mapped, a glyph is drawn and the desktop wants
    /// motion. Unmapped or with animations off, the timer is removed; with
    /// animations off the graphic is drawn at its first frame.
    fn run_clock(&self) {
        let timer = Rc::clone(&self.timer);
        let start = {
            let paint = Rc::clone(&self.paint);
            let timer = Rc::clone(&timer);
            let canvas = self.canvas.downgrade();
            move || {
                let Some(area) = canvas.upgrade() else { return };
                if let Some(id) = timer.borrow_mut().take() {
                    id.remove();
                }
                // Held, too, while its window is minimised or suspended:
                // the Mac's timelines stop when their window cannot be seen.
                if !area.is_mapped()
                    || !animations_enabled()
                    || paint.mask.borrow().is_none()
                    || super::glass_motion::window_hidden(&area)
                {
                    paint.time.set(0.0);
                    paint.epoch.set(None);
                    area.queue_draw();
                    return;
                }
                let interval =
                    std::time::Duration::from_secs_f64(1.0 / hero_art::field::FRAMES_PER_SECOND);
                let paint = Rc::clone(&paint);
                let weak = area.downgrade();
                let id = glib::timeout_add_local(interval, move || {
                    let Some(area) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let now = glib::monotonic_time();
                    let epoch = *paint.epoch.get().get_or_insert(now);
                    paint.epoch.set(Some(epoch));
                    paint.last.set(now);
                    paint
                        .time
                        .set(((now - epoch) as f64 / 1_000_000.0) % 3600.0);
                    area.queue_draw();
                    glib::ControlFlow::Continue
                });
                *timer.borrow_mut() = Some(id);
            }
        };
        let start = Rc::new(start);
        {
            let start = Rc::clone(&start);
            self.canvas.connect_map(move |_| start());
        }
        {
            let start = Rc::clone(&start);
            super::glass_motion::on_window_state(&self.canvas, move |_| start());
        }
        {
            let timer = Rc::clone(&timer);
            self.canvas.connect_unmap(move |_| {
                if let Some(id) = timer.borrow_mut().take() {
                    id.remove();
                }
            });
        }
        if let Some(settings) = gtk::Settings::default() {
            let start = Rc::clone(&start);
            settings.connect_gtk_enable_animations_notify(move |_| start());
        }
        *self.restart.borrow_mut() = Some(start);
    }

    /// Whether the graphic's timer is running: for a test.
    pub fn is_ticking(&self) -> bool {
        self.timer.borrow().is_some()
    }
}

/// The Insights hero's bars as widgets: each tool's name over its bar, the
/// longest first (`HomeInsightsGraphic`).
pub fn insights_bars(bars: &[(String, u64)], name: impl Fn(&str) -> String) -> gtk::Box {
    use hero_art::bars as b;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, b::SPACING);
    let most = bars.first().map_or(1, |(_, tokens)| *tokens);
    for (tool, tokens) in bars {
        let column = gtk::Box::new(gtk::Orientation::Vertical, b::NAME_GAP);
        column.set_valign(gtk::Align::End);
        let label = gtk::Label::builder()
            .label(name(tool))
            .single_line_mode(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .width_request(b::NAME_WIDTH)
            .max_width_chars(1)
            .build();
        label.add_css_class("tc-type-micro");
        label.add_css_class("tc-hero-bar-name");
        column.append(&label);
        let height = b::height(*tokens, most);
        let bar = gtk::DrawingArea::builder()
            .content_width(b::BAR_WIDTH as i32)
            .content_height(height.round() as i32)
            .halign(gtk::Align::Center)
            .build();
        bar.add_css_class("tc-hero-bar");
        bar.set_draw_func(|_, cr, width, height| {
            b::paint_bar(cr, width as f64, height as f64);
        });
        column.append(&bar);
        row.append(&column);
    }
    row.set_valign(gtk::Align::End);
    row
}

/// Where the bars sit on a card of `width` by `height`: centred in the
/// graphic's column at the trailing edge, then pushed right and down by
/// `bars::OFFSET`. As on the Mac, the graphic's column takes the taller of
/// the card and the graphic and is centred on the card: a graphic shorter
/// than the card stands on its bottom edge, a taller one hangs as far
/// below it as above.
pub fn place_bars(width: i32, height: i32, graphic_width: i32, graphic_height: i32) -> (i32, i32) {
    let centre = width as f64 - card::GRAPHIC_WIDTH / 2.0 + hero_art::bars::OFFSET.0;
    let feet = (height as f64 + height.max(graphic_height) as f64) / 2.0;
    (
        (centre - graphic_width as f64 / 2.0).round() as i32,
        (feet + hero_art::bars::OFFSET.1).floor() as i32 - graphic_height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bars_stand_where_the_macs_do() {
        // A 600 by 156 card, four bars 52 wide 14 apart, 214 tall with
        // their names: centred on 600 - 105 + 30. Taller than the card, the
        // graphic's frame takes its height and is centred on the card's,
        // so its feet are (156 + 214) / 2 below the top, then 40 more.
        let (x, y) = place_bars(600, 156, 250, 214);
        assert_eq!(x, 525 - 125);
        assert_eq!(y, (156 + 214) / 2 + 40 - 214);
        // Shorter than the card, its feet are on the card's bottom edge.
        assert_eq!(place_bars(600, 156, 250, 100).1, 156 + 40 - 100);
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn a_hero_is_one_button_that_hovers_and_closes_on_its_own() {
        gtk::init().unwrap();
        let hero = HeroCard::new(
            Ground::Campaign,
            "Title",
            Some("Line"),
            Some("Act"),
            Some("Close"),
        );
        hero.set_symbol(Some("cpu"));
        assert!(hero.pill.is_some());
        let close = hero.close.as_ref().expect("a closable card has an X");
        // The X is not inside the card's button: a control of its own.
        assert!(!close.is_ancestor(&hero.button));
        assert!(close.is_ancestor(&hero.root));
        assert!(!hero.is_hovered());
        hero.set_hover(true);
        assert!(hero.is_hovered());
        let plain = HeroCard::new(Ground::Missions, "Title", None, None, None);
        assert!(plain.close.is_none() && plain.pill.is_none());
        assert!(!plain.subtitle.is_visible());
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_the_graphic_ticks_only_while_shown_and_motion_is_wanted() {
        // libadwaita first, as the app starts it: the card's drawing
        // reads the style manager, which must not be created mid-frame.
        adw::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(true);
        let hero = HeroCard::new(Ground::Missions, "Title", None, Some("Act"), None);
        hero.set_symbol(Some("scope"));
        assert!(!hero.is_ticking(), "nothing ticks before the card is shown");
        let window = gtk::Window::builder()
            .default_width(600)
            .default_height(200)
            .child(&hero.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        while !hero.canvas.is_mapped() {
            context.iteration(true);
        }
        assert!(hero.is_ticking());
        // Reduced motion: the timer is removed and the first frame drawn.
        settings.set_gtk_enable_animations(false);
        assert!(!hero.is_ticking());
        assert_eq!(hero.time(), 0.0);
        settings.set_gtk_enable_animations(true);
        assert!(hero.is_ticking());
        // No glyph, nothing to move.
        hero.set_symbol(None);
        assert!(!hero.is_ticking());
        hero.set_symbol(Some("scope"));
        assert!(hero.is_ticking());
        // Hidden: the timer stops with the map.
        hero.root.set_visible(false);
        while hero.canvas.is_mapped() {
            context.iteration(true);
        }
        assert!(!hero.is_ticking());
        window.close();
        settings.set_gtk_enable_animations(previous);
    }

    /// `GlassHeroCard`: the hover wash fades over `motion.fast` on the
    /// token curve (`withAnimation(GlassMotion.fast) { hovering = inside }`),
    /// its amount `card::HOVER` (6%) of heroInk at the end.
    #[test]
    fn the_hover_wash_fades_over_the_macs_fast_curve() {
        let swift = include_str!("../../../../macos/Sources/TCDesign/Components/HeroCard.swift");
        assert!(
            swift.contains("withAnimation(GlassMotion.fast(reduceMotion)) { hovering = inside }")
        );
        assert!(swift.contains("shape.fill(GlassTokens.Color.heroInk.color.opacity(0.06))"));
        assert_eq!(HOVER_FADE, super::super::style::brand_tokens::motion::FAST);
        assert_eq!(hero_art::wash_alpha(0.0), 0.0);
        assert!((hero_art::wash_alpha(1.0) - 0.06).abs() < 1e-12);
        assert!((hero_art::wash_alpha(0.5) - 0.03).abs() < 1e-12);
    }

    /// Entering fades the wash through the middle to whole; with motion off
    /// it is whole at once.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_the_hover_wash_fades_in_and_out() {
        gtk::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(true);
        let hero = HeroCard::new(Ground::Campaign, "Title", None, Some("Act"), None);
        let window = gtk::Window::new();
        window.set_child(Some(&hero.root));
        window.present();
        let context = glib::MainContext::default();
        let pump_until = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done() && std::time::Instant::now() < deadline {
                context.iteration(false);
            }
        };
        pump_until(&|| hero.canvas.is_mapped());
        hero.hover_to(true);
        let between = Cell::new(false);
        pump_until(&|| {
            let w = hero.wash();
            between.set(between.get() || (w > 0.0 && w < 1.0));
            w == 1.0
        });
        assert!(between.get(), "the wash snapped on");
        hero.hover_to(false);
        pump_until(&|| hero.wash() == 0.0);
        assert_eq!(hero.wash(), 0.0);
        settings.set_gtk_enable_animations(false);
        hero.hover_to(true);
        assert_eq!(hero.wash(), 1.0, "motion off: at once");
        settings.set_gtk_enable_animations(true);
        window.destroy();
    }
}
