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

/// Keep every document block, including each table cell, in reading order.
/// Text is not interpreted as Pango markup; links remain visible in full.
fn document_text(document: &TermsDocument) -> String {
    let mut blocks = vec![document.title.clone(), document.effective.clone()];
    blocks.extend(document.blocks.iter().map(|block| {
        match block {
            TermsBlock::Heading { text }
            | TermsBlock::Paragraph { text }
            | TermsBlock::Callout { text } => text.clone(),
            TermsBlock::List { items } => items
                .iter()
                .map(|item| format!("• {item}"))
                .collect::<Vec<_>>()
                .join("\n"),
            TermsBlock::Table { header, rows } => std::iter::once(header.join(" | "))
                .chain(rows.iter().map(|row| row.join(" | ")))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }));
    blocks.join("\n\n")
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
            reader
                .reader
                .buffer()
                .set_text(&document_text(&fetched.document));
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

impl TermsPage {
    pub(super) fn new(on_continue: impl Fn() + 'static) -> Self {
        let copy = trace_commons_contributor::first_run_copy::first_run_copy();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let title = gtk::Label::new(Some(&format!(
            "{} {}",
            copy.welcome.legal.title_light, copy.welcome.legal.terms
        )));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.add_css_class("title-1");
        root.append(&title);
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
            reader: text,
            scroll,
            status,
            accept: accept.clone(),
            next: next.clone(),
            retry: retry.clone(),
        });
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
}
