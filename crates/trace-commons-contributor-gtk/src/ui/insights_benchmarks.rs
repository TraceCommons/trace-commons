//! Read-only native Benchmarks over the existing daemon worker.
use super::{copy, label};
use adw::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};

const CATEGORIES: [&str; 8] = [
    "bug_fix",
    "feature",
    "refactor",
    "tests",
    "docs",
    "understand",
    "review",
    "setup",
];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Selection {
    long: bool,
    spawned: bool,
}
impl Selection {
    fn weeks(self) -> u32 {
        if self.long { 13 } else { 4 }
    }
    fn population(self) -> &'static str {
        if self.spawned { "spawned" } else { "own" }
    }
    fn params(self) -> Value {
        json!({"weeks":self.weeks(), "population":self.population()})
    }
}
#[derive(Default)]
struct Flight {
    generation: u64,
    busy: bool,
    again: bool,
    active: bool,
}
impl Flight {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.again = false;
    }
    fn begin(&mut self) -> Option<u64> {
        self.invalidate();
        if !self.active {
            return None;
        }
        if self.busy {
            self.again = true;
            return None;
        }
        self.busy = true;
        Some(self.generation)
    }
    fn accepts(&self, generation: u64) -> bool {
        self.active && self.generation == generation
    }
}
#[derive(Deserialize)]
struct Model {
    label: String,
    tool: String,
    sessions: u32,
    kept_permille: Option<u32>,
    usd_micros_median: Option<u64>,
    active_secs_median: Option<u64>,
}
#[derive(Deserialize)]
struct Entry {
    label: String,
    figure: u64,
}
#[derive(Deserialize)]
struct Pick {
    leader: Entry,
    next: Entry,
}
#[derive(Deserialize)]
struct Category {
    key: String,
    sessions: u32,
    models: Vec<Model>,
    quality: Option<Pick>,
    value: Option<Pick>,
    speed: Option<Pick>,
}
#[derive(Deserialize)]
struct Answer {
    enabled: bool,
    readable: bool,
    feed: String,
    weeks: u32,
    population: String,
    min_sessions: u32,
    categories: Vec<Category>,
}
fn decode(value: Value, selected: Selection) -> Option<Answer> {
    let answer: Answer = serde_json::from_value(value).ok()?;
    (answer.enabled
        && answer.readable
        && answer.feed == "counter_pass"
        && answer.weeks == selected.weeks()
        && answer.population == selected.population())
    .then_some(answer)
}
fn fill(key: &str, values: &[(&str, String)]) -> String {
    values.iter().fold(copy(key).to_owned(), |s, (k, v)| {
        s.replace(&format!("{{{k}}}"), v)
    })
}
fn traces(n: u64) -> String {
    fill("analytics_bench_traces", &[("n", n.to_string())])
}
fn percent(n: u64) -> String {
    ((u128::from(n) + 5) / 10).to_string()
}
fn duration(n: u64) -> String {
    let minutes = (n / 60).max(1);
    if minutes < 60 {
        format!("{minutes}m")
    } else if minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{}h {}m", minutes / 60, minutes % 60)
    }
}
fn figure(kind: &str, n: u64) -> String {
    match kind {
        "quality" => fill("analytics_bench_kept", &[("p", percent(n))]),
        "value" => fill(
            "analytics_bench_per_trace",
            &[("x", super::live::dollars(n))],
        ),
        _ => duration(n),
    }
}
fn rows(category: &Category) -> Vec<&Model> {
    let mut models: Vec<_> = category.models.iter().collect();
    models.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.label.cmp(&b.label))
    });
    models
}
fn figures(model: &Model) -> [String; 3] {
    [
        model
            .kept_permille
            .map(|n| format!("{}%", percent(u64::from(n)))),
        model.usd_micros_median.map(super::live::dollars),
        model.active_secs_median.map(duration),
    ]
    .map(|s| s.unwrap_or_else(|| copy("analytics_unavailable").to_owned()))
}
fn heading(text: &str) -> gtk::Label {
    let l = label(text);
    l.add_css_class("heading");
    l
}
fn card() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("tc-insights-card");
    b
}
fn clear(body: &gtk::Box) {
    while let Some(c) = body.first_child() {
        body.remove(&c);
    }
}

pub(super) struct Benchmarks {
    pub root: gtk::ScrolledWindow,
    body: gtk::Box,
    selection: Cell<Selection>,
    flight: RefCell<Flight>,
    app: RefCell<Weak<super::super::App>>,
}
impl Benchmarks {
    pub(super) fn new() -> Rc<Self> {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        for setter in [
            gtk::prelude::WidgetExt::set_margin_top,
            gtk::prelude::WidgetExt::set_margin_bottom,
            gtk::prelude::WidgetExt::set_margin_start,
            gtk::prelude::WidgetExt::set_margin_end,
        ] {
            setter(&page, 20);
        }
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let population = gtk::DropDown::from_strings(&[
            copy("analytics_bench_population_own"),
            copy("analytics_bench_population_spawned"),
        ]);
        population.update_property(&[gtk::accessible::Property::Label(copy(
            "analytics_bench_population",
        ))]);
        let periods = [4, 13].map(|n| fill("analytics_bench_weeks", &[("n", n.to_string())]));
        let weeks = gtk::DropDown::from_strings(&[&periods[0], &periods[1]]);
        weeks.update_property(&[gtk::accessible::Property::Label(copy(
            "analytics_tab_benchmarks",
        ))]);
        controls.append(&population);
        controls.append(&weeks);
        page.append(&controls);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
        page.append(&body);
        let view = Rc::new(Self {
            root: gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&page)
                .build(),
            body,
            selection: Cell::new(Selection::default()),
            flight: RefCell::new(Flight::default()),
            app: RefCell::new(Weak::new()),
        });
        for (picker, is_weeks) in [(weeks, true), (population, false)] {
            let weak = Rc::downgrade(&view);
            picker.connect_selected_notify(move |picker| {
                let Some(view) = weak.upgrade() else { return };
                let mut selected = view.selection.get();
                if is_weeks {
                    selected.long = picker.selected() == 1;
                } else {
                    selected.spawned = picker.selected() == 1;
                }
                view.selection.set(selected);
                view.load();
            });
        }
        view
    }
    pub(super) fn bind_app(&self, app: &Rc<super::super::App>) {
        *self.app.borrow_mut() = Rc::downgrade(app);
    }
    pub(super) fn suspend(&self) {
        let mut f = self.flight.borrow_mut();
        f.active = false;
        f.invalidate();
    }
    pub(super) fn show(self: &Rc<Self>) {
        self.flight.borrow_mut().active = true;
        self.load();
    }
    fn load(self: &Rc<Self>) {
        if !self.flight.borrow().active {
            return;
        }
        clear(&self.body);
        let Some(app) = self.app.borrow().upgrade() else {
            self.body.append(&label(copy("analytics_unavailable")));
            return;
        };
        let spinner = gtk::Spinner::new();
        spinner.start();
        self.body.append(&spinner);
        let Some(generation) = self.flight.borrow_mut().begin() else {
            return;
        };
        let selected = self.selection.get();
        let weak = Rc::downgrade(self);
        app.call(
            "insights_benchmarks",
            selected.params(),
            move |_, result| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                let (again, accept) = {
                    let mut f = view.flight.borrow_mut();
                    f.busy = false;
                    let again = f.again && f.active;
                    f.again = false;
                    (again, f.accepts(generation))
                };
                if again {
                    view.load();
                    return;
                }
                if !accept {
                    return;
                }
                clear(&view.body);
                match result.ok().and_then(|v| decode(v, selected)) {
                    Some(answer) => view.present(answer),
                    None => view.body.append(&label(copy("analytics_unavailable"))),
                }
            },
        );
    }
    fn present(&self, answer: Answer) {
        let compared: u64 = answer
            .categories
            .iter()
            .map(|c| u64::from(c.sessions))
            .sum();
        self.body.append(&heading(&fill(
            if answer.population == "spawned" {
                "analytics_bench_intro_spawned"
            } else {
                "analytics_bench_intro"
            },
            &[("n", compared.to_string())],
        )));
        let ordered: Vec<_> = CATEGORIES
            .iter()
            .filter_map(|key| {
                answer
                    .categories
                    .iter()
                    .find(|c| c.key == *key && c.sessions > 0)
            })
            .collect();
        if ordered.is_empty() {
            self.body.append(&label(copy("analytics_bench_not_enough")));
            return;
        }
        self.mix(&ordered);
        for category in ordered {
            let body = card();
            body.append(&heading(copy(&format!(
                "analytics_bench_cat_{}",
                category.key
            ))));
            body.append(&label(&traces(u64::from(category.sessions))));
            let mut has_pick = false;
            for (kind, pick) in [
                ("quality", &category.quality),
                ("value", &category.value),
                ("speed", &category.speed),
            ] {
                let Some(pick) = pick else {
                    continue;
                };
                has_pick = true;
                let title = heading(copy(&format!("analytics_bench_{kind}")));
                title.set_tooltip_text(Some(copy(&format!("analytics_bench_{kind}_detail"))));
                body.append(&title);
                body.append(&label(&format!(
                    "{} · {}",
                    pick.leader.label,
                    figure(kind, pick.leader.figure)
                )));
                body.append(&label(&fill(
                    "analytics_bench_next",
                    &[
                        ("label", pick.next.label.clone()),
                        ("x", figure(kind, pick.next.figure)),
                    ],
                )));
            }
            if !category.models.is_empty() {
                body.append(&heading(copy("analytics_bench_yours")));
            }
            let grid = gtk::Grid::builder()
                .column_spacing(16)
                .row_spacing(8)
                .build();
            for (i, key) in ["kept", "cost", "time"].iter().enumerate() {
                grid.attach(
                    &label(copy(&format!("analytics_bench_col_{key}"))),
                    i as i32 + 1,
                    0,
                    1,
                    1,
                );
            }
            for (row, model) in rows(category).iter().enumerate() {
                let row = row as i32 + 1;
                grid.attach(
                    &label(&format!(
                        "{}\n{}",
                        model.label,
                        traces(u64::from(model.sessions))
                    )),
                    0,
                    row,
                    1,
                    1,
                );
                for (col, value) in figures(model).iter().enumerate() {
                    grid.attach(&label(value), col as i32 + 1, row, 1, 1);
                }
            }
            body.append(&grid);
            if !has_pick {
                body.append(&label(&fill(
                    "analytics_bench_compare_later",
                    &[("n", answer.min_sessions.to_string())],
                )));
            }
            self.body.append(&body);
        }
    }
    fn mix(&self, categories: &[&Category]) {
        let mut models: BTreeMap<(&str, &str), Vec<(&str, u32)>> = BTreeMap::new();
        for category in categories {
            for model in &category.models {
                if model.sessions > 0 {
                    models
                        .entry((&model.label, &model.tool))
                        .or_default()
                        .push((&category.key, model.sessions));
                }
            }
        }
        let mut models: Vec<_> = models
            .into_iter()
            .map(|(model, tasks)| {
                let total: u64 = tasks.iter().map(|(_, n)| u64::from(*n)).sum();
                (model, tasks, total)
            })
            .collect();
        models.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.0.cmp(b.0.0)));
        if models.is_empty() {
            return;
        }
        let body = card();
        body.append(&heading(copy("analytics_bench_mix_title")));
        for ((name, _), mut tasks, total) in models {
            body.append(&heading(name));
            body.append(&label(&traces(total)));
            tasks.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            for (key, n) in tasks {
                body.append(&label(copy(&format!("analytics_bench_cat_{key}"))));
                let bar = gtk::LevelBar::for_interval(0.0, 1.0);
                for offset in ["low", "high", "full"] {
                    bar.remove_offset_value(Some(offset));
                }
                bar.set_value(f64::from(n) / total as f64);
                body.append(&bar);
                body.append(&label(&fill(
                    "analytics_bench_mix_share",
                    &[
                        ("p", ((u64::from(n) * 100 + total / 2) / total).to_string()),
                        ("n", traces(u64::from(n))),
                    ],
                )));
            }
        }
        self.body.append(&body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesces_and_rejects_stale_or_hidden_results() {
        let mut f = Flight {
            active: true,
            ..Default::default()
        };
        let first = f.begin().unwrap();
        assert!(f.begin().is_none());
        assert!(f.again);
        assert!(!f.accepts(first));
        f.busy = false;
        let last = f.begin().unwrap();
        assert!(f.accepts(last));
        f.active = false;
        f.invalidate();
        assert!(!f.accepts(last));
        assert!(f.begin().is_none());
    }
    #[test]
    fn response_requires_readable_matching_live_selection() {
        let v = json!({"enabled":true,"readable":true,"feed":"counter_pass","weeks":4,"population":"own","min_sessions":5,"categories":[]});
        assert!(decode(v.clone(), Selection::default()).is_some());
        for (key, bad) in [
            ("enabled", json!(false)),
            ("readable", json!(false)),
            ("feed", json!("saved")),
            ("weeks", json!(13)),
            ("population", json!("spawned")),
            ("categories", json!({})),
        ] {
            let mut x = v.clone();
            x[key] = bad;
            assert!(decode(x, Selection::default()).is_none(), "{key}");
        }
    }
    #[test]
    fn rows_start_at_one_trace_without_inventing_picks() {
        let c:Category=serde_json::from_value(json!({"key":"tests","sessions":3,"models":[{"label":"z","tool":"codex","sessions":1},{"label":"a","tool":"codex","sessions":2}]})).unwrap();
        assert_eq!(rows(&c)[0].label, "a");
        assert!(c.quality.is_none() && c.value.is_none() && c.speed.is_none());
        assert_eq!(
            figures(&c.models[0]),
            [
                copy("analytics_unavailable").to_owned(),
                copy("analytics_unavailable").to_owned(),
                copy("analytics_unavailable").to_owned()
            ]
        );
    }
    #[test]
    fn known_zero_and_rounding_match_native() {
        let m:Model=serde_json::from_value(json!({"label":"a","tool":"codex","sessions":1,"kept_permille":995,"usd_micros_median":0,"active_secs_median":0})).unwrap();
        assert_eq!(figures(&m), ["100%", "$0.00", "1m"]);
        assert_eq!(duration(3900), "1h 5m");
        assert_eq!(percent(994), "99");
        assert_eq!(
            Selection {
                long: true,
                spawned: true
            }
            .params(),
            json!({"weeks":13,"population":"spawned"})
        );
    }
}
