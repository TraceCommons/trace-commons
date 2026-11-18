//! The published terms, read before Join. No enrollment or acceptance is sent
//! here: the first-run runner hands the exact accepted document to the daemon.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use trace_commons_contributor::terms::{self, FetchedTerms};
use trace_commons_protocol::terms::{TermsBlock, TermsDocument};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Accepted {
    pub version: String,
    pub content_sha256: String,
    pub accepted_at: String,
}

impl Accepted {
    pub(super) fn params(&self) -> serde_json::Value {
        serde_json::json!({"version": self.version, "content_sha256": self.content_sha256,
            "accepted_at": self.accepted_at})
    }
}

fn matches(accepted: Option<&Accepted>, loaded: Option<&FetchedTerms>) -> bool {
    match (accepted, loaded) {
        (Some(accepted), Some(loaded)) => {
            accepted.version == loaded.document.version
                && accepted.content_sha256 == loaded.content_sha256
        }
        _ => false,
    }
}

/// Every document block, including each table cell, in reading order, as
/// runs of inline text. Text is not interpreted as Pango markup; a link
/// reads as its label and opens in the browser, as on the Mac.
fn document_runs(document: &TermsDocument) -> Vec<(String, Inline)> {
    let plain = |text: &str| (text.to_owned(), Inline::Plain);
    let lines = |cells: &[String], separator: &str| {
        let mut runs = Vec::new();
        for (index, cell) in cells.iter().enumerate() {
            if index > 0 {
                runs.push(plain(separator));
            }
            runs.extend(inline_runs(cell));
        }
        runs
    };
    let mut blocks = vec![
        vec![plain(&document.title)],
        vec![plain(&document.effective)],
    ];
    blocks.extend(document.blocks.iter().map(|block| {
        match block {
            TermsBlock::Heading { text } => inline_runs(text)
                .into_iter()
                .map(|(text, style)| match style {
                    Inline::Plain => (text, Inline::Strong),
                    other => (text, other),
                })
                .collect(),
            TermsBlock::Paragraph { text } | TermsBlock::Callout { text } => inline_runs(text),
            TermsBlock::List { items } => {
                let mut runs = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        runs.push(plain("\n"));
                    }
                    runs.push(plain("• "));
                    runs.extend(inline_runs(item));
                }
                runs
            }
            TermsBlock::Table { header, rows } => {
                let mut runs = lines(header, " | ");
                for row in rows {
                    runs.push(plain("\n"));
                    runs.extend(lines(row, " | "));
                }
                runs
            }
        }
    }));
    let mut runs = Vec::new();
    for (index, block) in blocks.into_iter().enumerate() {
        if index > 0 {
            runs.push(plain("\n\n"));
        }
        runs.extend(block);
    }
    runs
}

#[cfg(test)]
fn document_text(document: &TermsDocument) -> String {
    document_runs(document)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

/// Write `runs` into `view`, styled, and return each link's character
/// range and URL.
fn fill(view: &gtk::TextView, runs: &[(String, Inline)]) -> Vec<(i32, i32, String)> {
    let buffer = view.buffer();
    buffer.set_text("");
    let table = buffer.tag_table();
    let tag = |name: &str, style: &dyn Fn(&gtk::TextTag)| {
        table.lookup(name).unwrap_or_else(|| {
            let tag = gtk::TextTag::new(Some(name));
            style(&tag);
            table.add(&tag);
            tag
        })
    };
    let mut links = Vec::new();
    for (text, style) in runs {
        let start = buffer.end_iter().offset();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, text);
        let end = buffer.end_iter().offset();
        let name = match style {
            Inline::Plain => continue,
            Inline::Strong => "strong",
            Inline::Emphasis => "emphasis",
            Inline::Code => "code",
            Inline::Link(url) => {
                links.push((start, end, url.clone()));
                "link"
            }
        };
        let tag = tag(name, &|tag: &gtk::TextTag| match name {
            "strong" => tag.set_weight(700),
            "emphasis" => tag.set_style(gtk::pango::Style::Italic),
            "code" => tag.set_family(Some("monospace")),
            _ => tag.set_underline(gtk::pango::Underline::Single),
        });
        buffer.apply_tag(
            &tag,
            &buffer.iter_at_offset(start),
            &buffer.iter_at_offset(end),
        );
    }
    links
}

/// The link at character `offset`, when it is a web address.
fn link_at(links: &[(i32, i32, String)], offset: i32) -> Option<&str> {
    links
        .iter()
        .find(|(start, end, _)| (*start..*end).contains(&offset))
        .map(|(_, _, url)| url.as_str())
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
}

/// One run of a terms block's inline text: the document's limited Markdown
/// (emphasis, strong, code spans, links), as the Mac reads it
/// (`TermsBlockView.inline`, inline-only, whitespace kept).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Inline {
    Plain,
    Strong,
    Emphasis,
    Code,
    /// A link, opened in the browser; its URL.
    Link(String),
}

/// `text`'s inline Markdown as runs. An unclosed marker is plain text.
fn inline_runs(text: &str) -> Vec<(String, Inline)> {
    let mut runs: Vec<(String, Inline)> = Vec::new();
    let mut plain = String::new();
    let push = |runs: &mut Vec<(String, Inline)>, plain: &mut String| {
        if !plain.is_empty() {
            runs.push((std::mem::take(plain), Inline::Plain));
        }
    };
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        if c == '\\' {
            if let Some(next) = after.chars().next() {
                plain.push(next);
                rest = &after[next.len_utf8()..];
                continue;
            }
        }
        if c == '[' {
            if let Some((label, url, tail)) = after.split_once("](").and_then(|(label, tail)| {
                let (url, tail) = tail.split_once(')')?;
                (!label.contains('[') && !url.is_empty()).then_some((label, url, tail))
            }) {
                push(&mut runs, &mut plain);
                runs.push((label.to_owned(), Inline::Link(url.to_owned())));
                rest = tail;
                continue;
            }
        }
        let marked = [
            ("**", Inline::Strong),
            ("*", Inline::Emphasis),
            ("`", Inline::Code),
        ]
        .into_iter()
        .find_map(|(mark, style)| {
            let inner = rest.strip_prefix(mark)?;
            let end = inner.find(mark)?;
            (end > 0).then(|| (inner[..end].to_owned(), style, &inner[end + mark.len()..]))
        });
        if let Some((inner, style, tail)) = marked {
            push(&mut runs, &mut plain);
            runs.push((inner, style));
            rest = tail;
            continue;
        }
        plain.push(c);
        rest = after;
    }
    push(&mut runs, &mut plain);
    runs
}

fn at_end(value: f64, upper: f64, page: f64) -> bool {
    value.is_finite()
        && upper.is_finite()
        && page.is_finite()
        && page > 0.0
        && upper > 0.0
        && value + page >= upper - 1.0
}

pub(super) struct TermsPage {
    pub root: gtk::Box,
    pub accepted: Rc<RefCell<Option<Accepted>>>,
}

struct Reader {
    root: glib::WeakRef<gtk::Box>,
    accepted: Rc<RefCell<Option<Accepted>>>,
    loaded: RefCell<Option<FetchedTerms>>,
    busy: Cell<bool>,
    pending: Cell<bool>,
    generation: Cell<u64>,
    ready_to_measure: Cell<bool>,
    reached_end: Cell<bool>,
    reader: gtk::TextView,
    links: RefCell<Vec<(i32, i32, String)>>,
    scroll: gtk::ScrolledWindow,
    status: gtk::Label,
    accept: gtk::CheckButton,
    next: gtk::Button,
    retry: gtk::Button,
}

impl Reader {
    fn mapped(&self) -> bool {
        self.root.upgrade().is_some_and(|root| root.is_mapped())
    }

    fn refresh(&self) {
        let loaded = self.loaded.borrow();
        let accepted = self.accepted.borrow();
        let matching = matches(accepted.as_ref(), loaded.as_ref());
        self.accept
            .set_sensitive(loaded.is_some() && (self.reached_end.get() || matching));
        self.next.set_sensitive(self.mapped() && matching);
    }

    fn measure(&self) {
        if !self.ready_to_measure.get() || self.loaded.borrow().is_none() || !self.mapped() {
            return;
        }
        let adjustment = self.scroll.vadjustment();
        let end = self.reader.iter_location(&self.reader.buffer().end_iter());
        let visible = self.reader.visible_rect();
        if visible.height() > 0
            && end.y() + end.height() <= visible.y() + visible.height() + 1
            && at_end(
                adjustment.value(),
                adjustment.upper(),
                adjustment.page_size(),
            )
        {
            self.reached_end.set(true);
        }
        self.refresh();
    }

    fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.pending.set(false);
        self.ready_to_measure.set(false);
        self.loaded.borrow_mut().take();
        self.next.set_sensitive(false);
        self.accept.set_sensitive(false);
    }

    fn load(self: &Rc<Self>) {
        if !self.mapped() {
            return;
        }
        self.invalidate();
        if self.busy.replace(true) {
            self.pending.set(true);
            return;
        }
        let generation = self.generation.get();
        let copy = trace_commons_contributor::first_run_copy::first_run_copy();
        self.status.set_text(copy.welcome.legal.loading);
        self.retry.set_visible(false);
        self.reader.buffer().set_text("");
        self.reached_end.set(false);
        // Preserve the acceptance while toggling off the old checkbox display.
        // Its signal only changes the receipt when a current document is loaded.
        self.accept.set_active(false);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(terms::fetch_published_blocking)
                .ok()
                .and_then(Result::ok);
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let fetched = rx.recv().await.ok().flatten();
            let Some(reader) = weak.upgrade() else { return };
            reader.busy.set(false);
            if reader.pending.replace(false) && reader.mapped() {
                reader.load();
                return;
            }
            if reader.generation.get() != generation || !reader.mapped() {
                return;
            }
            let copy = trace_commons_contributor::first_run_copy::first_run_copy();
            let Some(fetched) = fetched else {
                reader.status.set_text(copy.welcome.legal.failed);
                reader.retry.set_visible(true);
                reader.refresh();
                return;
            };
            reader.status.set_text(if fetched.document.in_effect {
                copy.welcome.legal.scroll_hint
            } else {
                copy.welcome.legal.not_in_effect
            });
            *reader.links.borrow_mut() = fill(&reader.reader, &document_runs(&fetched.document));
            let matching = matches(reader.accepted.borrow().as_ref(), Some(&fetched));
            if !matching {
                reader.accepted.borrow_mut().take();
            }
            *reader.loaded.borrow_mut() = Some(fetched);
            reader.accept.set_active(matching);
            reader.scroll.vadjustment().set_value(0.0);
            reader.refresh();
            // Do not mistake the previous/empty buffer's adjustment for the new
            // document's bottom. Let GTK allocate and lay out its wrapped text.
            let weak = Rc::downgrade(&reader);
            glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                if let Some(reader) = weak.upgrade()
                    && reader.generation.get() == generation
                    && reader.mapped()
                {
                    reader.ready_to_measure.set(true);
                    reader.measure();
                }
            });
        });
    }
}

/// The step's title: the light half then the bold one, joined as the Mac's
/// `FirstRunTitle` joins them (the light half ends in its own space).
fn title() -> String {
    let legal = trace_commons_contributor::first_run_copy::first_run_copy()
        .welcome
        .legal;
    [legal.title_light, legal.terms].concat()
}

impl TermsPage {
    pub(super) fn new(on_continue: impl Fn() + 'static) -> Self {
        let copy = trace_commons_contributor::first_run_copy::first_run_copy();
        // The same frame and title as every other step (`first_run_screens::page`).
        let root = super::first_run_screens::page(&title());
        let text = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(12)
            .right_margin(12)
            .top_margin(12)
            .bottom_margin(12)
            .build();
        text.update_property(&[gtk::accessible::Property::Label(copy.welcome.legal.terms)]);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .min_content_height(220)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&text)
            .build();
        root.append(&scroll);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_xalign(0.0);
        root.append(&status);
        let retry = gtk::Button::with_label(copy.welcome.legal.retry);
        retry.set_visible(false);
        root.append(&retry);
        let accept = gtk::CheckButton::with_label(copy.welcome.legal.accept);
        accept.set_sensitive(false);
        accept.set_tooltip_text(Some(copy.welcome.legal.scroll_hint));
        root.append(&accept);
        let next = gtk::Button::with_label(copy.frame.continue_button);
        next.add_css_class("suggested-action");
        next.set_halign(gtk::Align::End);
        next.set_sensitive(false);
        root.append(&next);
        let accepted = Rc::new(RefCell::new(None));
        let reader = Rc::new(Reader {
            root: root.downgrade(),
            accepted: accepted.clone(),
            loaded: RefCell::new(None),
            busy: Cell::new(false),
            pending: Cell::new(false),
            generation: Cell::new(0),
            ready_to_measure: Cell::new(false),
            reached_end: Cell::new(false),
            reader: text.clone(),
            links: RefCell::new(Vec::new()),
            scroll,
            status,
            accept: accept.clone(),
            next: next.clone(),
            retry: retry.clone(),
        });
        // A link in the terms opens in the browser, as on the Mac.
        let click = gtk::GestureClick::new();
        click.connect_released({
            let weak = Rc::downgrade(&reader);
            move |_, _, x, y| {
                let Some(reader) = weak.upgrade() else { return };
                let view = &reader.reader;
                let (bx, by) =
                    view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
                let Some(iter) = view.iter_at_location(bx, by) else {
                    return;
                };
                if let Some(url) = link_at(&reader.links.borrow(), iter.offset()) {
                    super::credential::open_browser(url);
                }
            }
        });
        text.add_controller(click);
        let state = reader.clone();
        root.connect_map(move |_| state.load());
        let weak = Rc::downgrade(&reader);
        root.connect_unmap(move |_| {
            if let Some(reader) = weak.upgrade() {
                reader.invalidate();
            }
        });
        let weak = Rc::downgrade(&reader);
        retry.connect_clicked(move |_| {
            if let Some(reader) = weak.upgrade() {
                reader.load();
            }
        });
        let weak = Rc::downgrade(&reader);
        accept.connect_toggled(move |check| {
            let Some(reader) = weak.upgrade() else { return };
            let loaded = reader.loaded.borrow();
            let Some(loaded) = loaded.as_ref() else {
                return;
            };
            if check.is_active()
                && (reader.reached_end.get()
                    || matches(reader.accepted.borrow().as_ref(), Some(loaded)))
            {
                // A preserved acceptance keeps the time of the actual action.
                if !matches(reader.accepted.borrow().as_ref(), Some(loaded)) {
                    *reader.accepted.borrow_mut() = Some(Accepted {
                        version: loaded.document.version.clone(),
                        content_sha256: loaded.content_sha256.clone(),
                        accepted_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
            } else {
                reader.accepted.borrow_mut().take();
            }
            reader.refresh();
        });
        let weak = Rc::downgrade(&reader);
        next.connect_clicked(move |_| {
            let Some(reader) = weak.upgrade() else { return };
            let can_continue = reader.mapped()
                && matches(
                    reader.accepted.borrow().as_ref(),
                    reader.loaded.borrow().as_ref(),
                );
            if can_continue {
                on_continue();
            }
        });
        for signal in [true, false] {
            let weak = Rc::downgrade(&reader);
            let measure = move |_: &gtk::Adjustment| {
                if let Some(reader) = weak.upgrade() {
                    reader.measure();
                }
            };
            if signal {
                reader.scroll.vadjustment().connect_value_changed(measure);
            } else {
                reader.scroll.vadjustment().connect_changed(measure);
            }
        }
        Self { root, accepted }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> FetchedTerms {
        FetchedTerms {
            content_sha256: "a".repeat(64),
            document: TermsDocument {
                schema_version: "trace_commons.terms.v1".into(),
                version: "v1".into(),
                effective: "2026-11-01".into(),
                in_effect: true,
                title: "Terms".into(),
                blocks: vec![TermsBlock::Paragraph {
                    text: "Read this".into(),
                }],
            },
        }
    }
    #[test]
    fn acceptance_requires_current_loaded_version_and_exact_hash() {
        let mut loaded = document();
        let accepted = Accepted {
            version: "v1".into(),
            content_sha256: loaded.content_sha256.clone(),
            accepted_at: "2026-11-01T00:00:00Z".into(),
        };
        assert!(matches(Some(&accepted), Some(&loaded)));
        assert!(!matches(Some(&accepted), None));
        loaded.content_sha256 = "b".repeat(64);
        assert!(!matches(Some(&accepted), Some(&loaded)));
        loaded.content_sha256 = accepted.content_sha256.clone();
        loaded.document.version = "v2".into();
        assert!(!matches(Some(&accepted), Some(&loaded)));
    }
    #[test]
    fn empty_allocation_cannot_unlock_acceptance() {
        assert!(!at_end(0.0, 0.0, 0.0));
        assert!(!at_end(0.0, 1000.0, 400.0));
        assert!(at_end(600.0, 1000.0, 400.0));
        assert!(at_end(0.0, 100.0, 400.0));
        assert!(!at_end(f64::NAN, 100.0, 400.0));
    }
    #[test]
    fn inline_markdown_reads_as_the_mac_reads_it() {
        let plain = |s: &str| (s.to_owned(), Inline::Plain);
        assert_eq!(inline_runs("no marks"), [plain("no marks")]);
        assert_eq!(
            inline_runs("See the [Privacy Policy](https://example.com/privacy) now."),
            [
                plain("See the "),
                (
                    "Privacy Policy".to_owned(),
                    Inline::Link("https://example.com/privacy".into())
                ),
                plain(" now."),
            ]
        );
        assert_eq!(
            inline_runs("a **b** *c* `d`"),
            [
                plain("a "),
                ("b".to_owned(), Inline::Strong),
                plain(" "),
                ("c".to_owned(), Inline::Emphasis),
                plain(" "),
                ("d".to_owned(), Inline::Code),
            ]
        );
        assert_eq!(
            inline_runs("an *unclosed mark"),
            [plain("an *unclosed mark")]
        );
        assert_eq!(
            inline_runs("[not a link] here"),
            [plain("[not a link] here")]
        );
        assert_eq!(
            inline_runs(r"a \*literal\* star"),
            [plain("a *literal* star")]
        );
    }

    #[test]
    fn only_a_web_address_opens() {
        let links = vec![
            (0, 4, "https://example.com".to_owned()),
            (10, 14, "file:///etc/passwd".to_owned()),
        ];
        assert_eq!(link_at(&links, 2), Some("https://example.com"));
        assert_eq!(link_at(&links, 4), None);
        assert_eq!(link_at(&links, 11), None, "not a web address");
        assert_eq!(link_at(&links, 20), None);
    }

    #[test]
    fn every_block_and_table_cell_is_presented() {
        let mut terms = document().document;
        terms.blocks = vec![
            TermsBlock::Heading {
                text: "Heading".into(),
            },
            TermsBlock::List {
                items: vec!["one".into(), "two".into()],
            },
            TermsBlock::Table {
                header: vec!["A".into(), "B".into()],
                rows: vec![vec!["C".into(), "D".into()]],
            },
            TermsBlock::Callout {
                text: "callout".into(),
            },
        ];
        let text = document_text(&terms);
        for word in ["Heading", "one", "two", "A | B", "C | D", "callout"] {
            assert!(text.contains(word));
        }
    }
    #[test]
    fn title_joins_its_halves_as_the_mac_does() {
        // `FirstRunTitle` concatenates; the light half carries its own space.
        let legal = trace_commons_contributor::first_run_copy::first_run_copy()
            .welcome
            .legal;
        assert_eq!(title(), [legal.title_light, legal.terms].concat());
        assert!(!title().contains("  "), "double space in {:?}", title());
    }
}

/// Mounted checks, run under a Linux display: `--ignored --test-threads=1`.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod mounted_tests {
    use super::*;

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn terms_sits_in_the_same_frame_as_the_other_steps() {
        adw::init().expect("GTK display unavailable");
        let page = TermsPage::new(|| {});
        let framed = super::super::first_run_screens::page("");
        assert_eq!(page.root.margin_start(), framed.margin_start());
        assert_eq!(page.root.margin_end(), framed.margin_end());
        assert_eq!(page.root.margin_top(), framed.margin_top());
        assert_eq!(page.root.margin_bottom(), framed.margin_bottom());
        let heading = page
            .root
            .first_child()
            .and_then(|w| w.downcast::<gtk::Label>().ok())
            .expect("title label first");
        assert!(heading.has_css_class("tc-flow-title"));
        assert_eq!(heading.label(), title());
        let next = page
            .root
            .last_child()
            .and_then(|w| w.downcast::<gtk::Button>().ok())
            .expect("continue last");
        assert_eq!(next.halign(), gtk::Align::End);
    }
}
