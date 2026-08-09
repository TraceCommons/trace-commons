//! Look inside, then decide.
//!
//! The design premise, from the shared spec: **never ask the contributor to
//! judge redaction quality.** They cannot, and showing redacted text beside
//! an Approve button asks for a rubber stamp. So the sheet answers the two
//! questions they can answer -- is this project OK to share at all, and is
//! there anything specific in here that must not leave -- and Search is the
//! first tab with the cursor already in it, because that is the highest
//! value affordance in the product: someone under an NDA gets certainty in
//! five seconds without reading 148 turns.
//!
//! `Contribute` exists here and nowhere else, and it is followed by an undo
//! counted against the daemon's own deadline.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::App;
use crate::copy;
use crate::model::{ApproveResult, PreviewSummary, human_bytes};

/// Open the preview sheet on the `index`-th pending entry.
pub fn open(app: &Rc<App>, index: usize) {
    let entries = app.entries.borrow();
    let pending: Vec<crate::model::QueueEntry> = entries
        .iter()
        .filter(|e| e.state == "pending")
        .cloned()
        .collect();
    drop(entries);
    if index >= pending.len() {
        return;
    }
    Sheet::present(app, pending, index);
}

struct Sheet {
    app: Rc<App>,
    window: adw::Window,
    title: adw::WindowTitle,
    pending: Vec<crate::model::QueueEntry>,
    index: RefCell<usize>,

    search_entry: gtk::SearchEntry,
    search_results: gtk::Box,
    search_summary: gtk::Label,
    recent_row: gtk::Box,

    whats_in_it: gtk::Box,
    body_view: gtk::TextView,
    permissions: gtk::Box,

    contribute: gtk::Button,
    /// The redacted body for the entry currently shown, when this
    /// deployment can serve one. See `backend`.
    body: RefCell<Option<String>>,
}

impl Sheet {
    fn present(app: &Rc<App>, pending: Vec<crate::model::QueueEntry>, index: usize) {
        let window = adw::Window::builder()
            .transient_for(&app.window)
            .modal(true)
            .default_width(900)
            .default_height(720)
            .build();

        let title = adw::WindowTitle::new("", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::None)
            .vexpand(true)
            .build();
        let switcher = gtk::StackSwitcher::builder().stack(&stack).build();
        switcher.set_margin_top(6);
        switcher.set_halign(gtk::Align::Center);

        // 1. Search, first and focused.
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Anything that must not leave this machine"));
        let search_summary = gtk::Label::builder().xalign(0.0).wrap(true).build();
        let search_results = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let recent_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let search_page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let search_prompt = gtk::Label::builder()
            .label(copy::SEARCH_PROMPT)
            .xalign(0.0)
            .wrap(true)
            .build();
        search_page.append(&search_prompt);
        search_page.append(&search_entry);
        search_page.append(&recent_row);
        search_page.append(&search_summary);
        let results_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&search_results)
            .build();
        search_page.append(&results_scroller);
        stack.add_titled(&search_page, Some("search"), copy::TAB_SEARCH);

        // 2. What's in it.
        let whats_in_it = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let whats_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&whats_in_it)
            .build();
        stack.add_titled(&whats_scroller, Some("whats-in-it"), copy::TAB_WHATS_IN_IT);

        // 3. Exactly what would be sent.
        let body_view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .left_margin(12)
            .right_margin(12)
            .top_margin(12)
            .bottom_margin(12)
            .build();
        let body_scroller = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&body_view)
            .build();
        stack.add_titled(
            &body_scroller,
            Some("would-be-sent"),
            copy::TAB_WOULD_BE_SENT,
        );

        // 4. Permissions.
        let permissions = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let permissions_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&permissions)
            .build();
        stack.add_titled(
            &permissions_scroller,
            Some("permissions"),
            copy::TAB_PERMISSIONS,
        );

        stack.set_visible_child_name("search");

        let skip = gtk::Button::with_label(copy::NOT_THIS_ONE);
        skip.set_tooltip_text(Some(copy::NOT_THIS_ONE_TOOLTIP));
        let contribute = gtk::Button::with_label(copy::CONTRIBUTE);
        contribute.add_css_class("suggested-action");
        contribute.set_sensitive(false);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let actions = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        actions.append(&skip);
        actions.append(&spacer);
        actions.append(&contribute);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&switcher);
        content.append(&stack);
        content.append(&actions);
        window.set_content(Some(&content));

        let sheet = Rc::new(Sheet {
            app: Rc::clone(app),
            window: window.clone(),
            title,
            pending,
            index: RefCell::new(index),
            search_entry: search_entry.clone(),
            search_results,
            search_summary,
            recent_row,
            whats_in_it,
            body_view,
            permissions,
            contribute: contribute.clone(),
            body: RefCell::new(None),
        });

        let s = Rc::clone(&sheet);
        search_entry.connect_search_changed(move |entry| {
            s.run_search(&entry.text(), false);
        });
        let s = Rc::clone(&sheet);
        search_entry.connect_activate(move |entry| {
            s.remember_search(&entry.text());
            s.run_search(&entry.text(), true);
        });
        let s = Rc::clone(&sheet);
        skip.connect_clicked(move |_| s.dismiss_current());
        let s = Rc::clone(&sheet);
        contribute.connect_clicked(move |_| s.approve_current());

        sheet.load();
        window.present();
        search_entry.grab_focus();
    }

    fn current(&self) -> Option<&crate::model::QueueEntry> {
        self.pending.get(*self.index.borrow())
    }

    /// Fetch the preview for the entry now showing.
    ///
    /// Deliberately re-previewed rather than read from the row cache: a
    /// preview pins the entry to the exact envelope it describes, and the
    /// approval that may follow covers those bytes. Approving against a
    /// summary fetched minutes ago would be approving something the daemon
    /// is no longer holding.
    fn load(self: &Rc<Self>) {
        let Some(entry) = self.current() else {
            self.window.close();
            return;
        };
        self.title.set_title(&entry.project_label);
        self.title.set_subtitle(&format!(
            "{} - {} of {}",
            entry.agent_label(),
            *self.index.borrow() + 1,
            self.pending.len()
        ));
        self.contribute.set_sensitive(false);
        self.search_summary.set_text("");
        self.clear_results();
        self.body_view
            .buffer()
            .set_text("Working out exactly what would be sent…");

        let sheet = Rc::clone(self);
        let entry_id = entry.entry_id.clone();
        let requested = entry_id.clone();
        self.app.preview(&requested, move |app, result| {
            match result {
                Ok((summary, body)) => {
                    app.previews
                        .borrow_mut()
                        .insert(entry_id.clone(), summary.clone());
                    sheet.fill(&summary, body);
                    super::queue::render(app);
                }
                Err(label) => sheet.fill_failure(&label),
            }
            sheet.render_recent();
        });
    }

    fn fill(self: &Rc<Self>, summary: &PreviewSummary, body: Option<String>) {
        // Approving is only allowed against a real, pinned preview. An
        // unenrolled build carries a placeholder identity and is not
        // bindable to an approval, so the button stays off and the sheet
        // says why.
        self.contribute.set_sensitive(summary.enrolled);

        *self.body.borrow_mut() = body.clone();
        match &body {
            Some(text) => {
                let buffer = self.body_view.buffer();
                buffer.set_text(text);
                highlight_redactions(&buffer, text);
            }
            None => self
                .body_view
                .buffer()
                .set_text(copy::BODY_NOT_AVAILABLE_HERE),
        }

        // "What's in it", from what the contract actually reports. Files
        // touched, tools invoked and the model are not on this response --
        // see the report's contract notes.
        while let Some(child) = self.whats_in_it.first_child() {
            self.whats_in_it.remove(&child);
        }
        for (heading, value) in [
            ("Turns", format!("{}", summary.event_count)),
            (
                "Would send",
                format!(
                    "{} (the session file on disk is {})",
                    human_bytes(summary.would_send_bytes),
                    human_bytes(summary.raw_session_bytes)
                ),
            ),
            ("Scrubbing found", summary.scrubbed_line()),
            (
                "Personal information spotted",
                if summary.pii_labels_present.is_empty() {
                    "None of the kinds it looks for".to_string()
                } else {
                    summary.pii_labels_present.join(", ")
                },
            ),
        ] {
            self.whats_in_it
                .append(&super::titled_paragraph(heading, &value));
        }
        self.whats_in_it.append(&super::titled_paragraph(
            "Residual risk",
            copy::RESIDUAL_RISK,
        ));
        if !summary.enrolled {
            self.whats_in_it.append(&super::titled_paragraph(
                "Not connected yet",
                copy::UNENROLLED_PREVIEW,
            ));
        }

        // Permissions, restated at the moment of consent rather than only
        // at onboarding.
        while let Some(child) = self.permissions.first_child() {
            self.permissions.remove(&child);
        }
        let intro = gtk::Label::builder()
            .label(copy::PERMISSIONS_INTRO)
            .xalign(0.0)
            .wrap(true)
            .build();
        self.permissions.append(&intro);
        let sheet = Rc::clone(self);
        let scopes = summary.consent_scopes.clone();
        self.app.call(
            "consent_options",
            serde_json::json!({}),
            move |_, result| {
                let described: Vec<crate::model::ConsentScope> = result
                    .ok()
                    .and_then(|v| serde_json::from_value(v.get("scopes").cloned()?).ok())
                    .unwrap_or_default();
                for name in &scopes {
                    let description = described
                        .iter()
                        .find(|s| &s.name == name)
                        .map(|s| s.description.clone())
                        .unwrap_or_default();
                    sheet
                        .permissions
                        .append(&super::titled_paragraph(name, &description));
                }
                let note = gtk::Label::builder()
                    .label(copy::PERMISSIONS_REQUESTED_NOTE)
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                note.add_css_class("dim-label");
                sheet.permissions.append(&note);
            },
        );

        if !self.search_entry.text().is_empty() {
            self.run_search(&self.search_entry.text(), false);
        }
    }

    fn fill_failure(self: &Rc<Self>, label: &str) {
        self.contribute.set_sensitive(false);
        let sentence = match label {
            "preview-failed" | "unavailable" => {
                "Trace Commons can't work out what would be sent right now, so there is nothing \
                 to decide on yet. Nothing has been sent."
            }
            "unknown-entry-id" | "session-file-vanished" => {
                "This session is no longer waiting. Nothing was sent."
            }
            _ => "Something went wrong working out what would be sent. Nothing has been sent.",
        };
        self.body_view.buffer().set_text(sentence);
        self.search_summary.set_text(sentence);
    }

    fn clear_results(&self) {
        while let Some(child) = self.search_results.first_child() {
            self.search_results.remove(&child);
        }
    }

    /// Search the redacted body.
    ///
    /// The answer a contributor wants is usually "0 matches", and getting it
    /// in one keystroke is the point. When this deployment cannot serve the
    /// body, the sheet says so rather than reporting a reassuring zero it
    /// has not earned.
    fn run_search(self: &Rc<Self>, needle: &str, remember: bool) {
        self.clear_results();
        let needle = needle.trim();
        if needle.is_empty() {
            self.search_summary.set_text("");
            return;
        }
        let body = self.body.borrow();
        let Some(body) = body.as_deref() else {
            self.search_summary.set_text(copy::BODY_NOT_AVAILABLE_HERE);
            return;
        };
        if remember {
            // no-op: remembering happens in `remember_search`, kept separate
            // so a keystroke-by-keystroke search does not fill the list.
        }

        let hay = body.to_lowercase();
        let pin = needle.to_lowercase();
        let hits: Vec<usize> = hay.match_indices(&pin).map(|(i, _)| i).collect();
        if hits.is_empty() {
            self.search_summary.set_text("0 matches");
            return;
        }
        self.search_summary.set_text(&format!(
            "{} {}",
            hits.len(),
            if hits.len() == 1 { "match" } else { "matches" }
        ));
        for start in hits.iter().take(50) {
            let snippet = context_around(body, *start, needle.len());
            let label = gtk::Label::builder()
                .label(snippet)
                .xalign(0.0)
                .wrap(true)
                .selectable(true)
                .build();
            label.add_css_class("monospace");
            self.search_results.append(&label);
        }
        if hits.len() > 50 {
            let more = gtk::Label::builder()
                .label(format!("…and {} more", hits.len() - 50))
                .xalign(0.0)
                .build();
            more.add_css_class("dim-label");
            self.search_results.append(&more);
        }
    }

    fn remember_search(self: &Rc<Self>, needle: &str) {
        let needle = needle.trim().to_string();
        if needle.is_empty() {
            return;
        }
        let mut recent = self.app.recent_searches.borrow_mut();
        recent.retain(|r| r != &needle);
        recent.insert(0, needle);
        recent.truncate(6);
        drop(recent);
        self.render_recent();
    }

    /// Recent searches, so the second trace is one click rather than one
    /// retyping of a client's name.
    fn render_recent(self: &Rc<Self>) {
        while let Some(child) = self.recent_row.first_child() {
            self.recent_row.remove(&child);
        }
        for term in self.app.recent_searches.borrow().iter() {
            let button = gtk::Button::with_label(term);
            button.add_css_class("pill");
            let sheet = Rc::clone(self);
            let term = term.clone();
            button.connect_clicked(move |_| {
                sheet.search_entry.set_text(&term);
                sheet.run_search(&term, false);
            });
            self.recent_row.append(&button);
        }
    }

    fn dismiss_current(self: &Rc<Self>) {
        let Some(entry) = self.current() else { return };
        let entry_id = entry.entry_id.clone();
        let sheet = Rc::clone(self);
        self.app.call(
            "dismiss",
            serde_json::json!({ "entry_id": entry_id }),
            move |app, _| {
                app.refresh();
                sheet.advance();
            },
        );
    }

    /// Approve exactly the bytes this sheet described, then offer a real
    /// undo.
    fn approve_current(self: &Rc<Self>) {
        let Some(entry) = self.current() else { return };
        let entry_id = entry.entry_id.clone();
        self.contribute.set_sensitive(false);
        let sheet = Rc::clone(self);
        self.app.call(
            "approve",
            serde_json::json!({ "entry_id": entry_id }),
            move |app, result| {
                match result {
                    Ok(value) => {
                        let approved: ApproveResult =
                            serde_json::from_value(value).unwrap_or(ApproveResult {
                                approved: 1,
                                hold_secs: 0,
                                hold_until: None,
                            });
                        offer_undo(app, &entry_id, approved);
                    }
                    Err(_) => {
                        app.toast("That couldn't be approved just now. Nothing has been sent.")
                    }
                }
                app.refresh();
                sheet.advance();
            },
        );
    }

    /// `Contribute` advances to the next entry in the sheet, so three
    /// sessions is three deliberate clicks in one flow. There is no
    /// select-all, and there never will be one here.
    fn advance(self: &Rc<Self>) {
        let next = *self.index.borrow() + 1;
        if next >= self.pending.len() {
            self.window.close();
            return;
        }
        *self.index.borrow_mut() = next;
        self.load();
    }
}

/// The undo window, counted against the daemon's clock.
///
/// `hold_until` is the instant the daemon will first consider the entry for
/// upload, read from the entry it just wrote. Counting down against a
/// duration this process picked instead would be the same bug as having no
/// hold at all: the countdown and the daemon would disagree about when the
/// decision stops being reversible. `hold_until: null` means no undo may be
/// offered, so none is.
fn offer_undo(app: &Rc<App>, entry_id: &str, approved: ApproveResult) {
    let Some(hold_until) = approved.hold_until else {
        app.toast(copy::APPROVED_NO_UNDO);
        return;
    };

    let toast = adw::Toast::new(copy::SENDING);
    toast.set_button_label(Some(copy::UNDO));
    // Dismissed by the countdown below, not by a timeout of its own: the
    // window that matters is the daemon's.
    toast.set_timeout(0);

    let entry_id_for_undo = entry_id.to_string();
    let app_for_undo = Rc::clone(app);
    toast.connect_button_clicked(move |toast| {
        toast.dismiss();
        let entry_id = entry_id_for_undo.clone();
        app_for_undo.call(
            "cancel",
            serde_json::json!({ "entry_id": entry_id }),
            |app, result| {
                match result {
                    Ok(_) => app.toast("Not sent. It's back in the queue."),
                    // `cancel` is guaranteed to succeed for the whole hold,
                    // so this is the rare late press.
                    Err(_) => app.toast("Too late to take that one back -- it has already gone."),
                }
                app.refresh();
            },
        );
    });

    app.toasts.add_toast(toast.clone());

    glib::timeout_add_seconds_local(1, move || {
        let remaining = (hold_until - chrono::Utc::now()).num_seconds();
        if remaining <= 0 {
            toast.dismiss();
            return glib::ControlFlow::Break;
        }
        toast.set_title(&format!("{} ({remaining})", copy::SENDING));
        glib::ControlFlow::Continue
    });
}

/// Show where scrubbing fired, rather than leaving holes.
///
/// The pipeline replaces removed values with visible markers
/// (`<PRIVATE_SECRET_1>`, `[REDACTED]`), so they are already in the text;
/// this makes them legible as chips instead of noise, which is what lets a
/// contributor see *where* redaction happened rather than only being told
/// how often.
fn highlight_redactions(buffer: &gtk::TextBuffer, text: &str) {
    let tag = buffer
        .create_tag(None, &[("weight", &700i32), ("background", &"#f6d32d")])
        .expect("creating a text tag");
    let mut byte = 0usize;
    while byte < text.len() {
        let rest = &text[byte..];
        let opener = rest.find('<').into_iter().chain(rest.find('[')).min();
        let Some(offset) = opener else { break };
        let start = byte + offset;
        let closer = if text[start..].starts_with('<') {
            '>'
        } else {
            ']'
        };
        let Some(end_offset) = text[start..].find(closer) else {
            break;
        };
        let end = start + end_offset + closer.len_utf8();
        let marker = &text[start..end];
        let is_redaction = marker.starts_with("<PRIVATE_") || marker.starts_with("[REDACTED");
        if is_redaction {
            let start_chars = text[..start].chars().count() as i32;
            let end_chars = text[..end].chars().count() as i32;
            let a = buffer.iter_at_offset(start_chars);
            let b = buffer.iter_at_offset(end_chars);
            buffer.apply_tag(&tag, &a, &b);
        }
        byte = end;
    }
}

/// A readable window around a search hit. Character-safe: slicing a UTF-8
/// body on a byte offset would panic on a multi-byte boundary, and traces
/// contain plenty of those.
fn context_around(body: &str, byte_start: usize, needle_len: usize) -> String {
    let start = body[..byte_start]
        .char_indices()
        .rev()
        .take(60)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(byte_start);
    let after = byte_start + needle_len;
    let end = body[after.min(body.len())..]
        .char_indices()
        .take(60)
        .last()
        .map(|(i, _)| after + i)
        .unwrap_or(body.len());
    let snippet = body[start..end.min(body.len())].replace('\n', " ");
    format!("…{}…", snippet.trim())
}

#[cfg(test)]
mod tests {
    use super::context_around;

    #[test]
    fn context_never_splits_a_multibyte_character() {
        let body = "prefix ünïcödé haystack needle tail ünïcödé more";
        let start = body.find("needle").unwrap();
        let snippet = context_around(body, start, "needle".len());
        assert!(snippet.contains("needle"));
    }
}
