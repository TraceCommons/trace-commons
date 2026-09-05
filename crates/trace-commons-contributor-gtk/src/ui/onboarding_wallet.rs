//! Native widget and browser adapter. Rust daemon owns the wallet state machine.
use super::App;
use super::onboarding::{Onboarding, Step, body_label, load_consent_options};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use trace_commons_contributor::witness_copy::witness_copy;
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct View {
    flow_id: String,
    state: String,
    busy: bool,
    can_check: bool,
    can_start: bool,
    can_edit: bool,
    can_cancel: bool,
    wait: bool,
    message: String,
    tone: String,
    glyph: String,
    browser_url: Option<String>,
}
struct Wallet {
    root: gtk::Box,
    commons: gtk::Entry,
    account: gtk::Entry,
    check: gtk::Button,
    start: gtk::Button,
    cancel: gtk::Button,
    message: gtk::Label,
    view: RefCell<View>,
    pending: Cell<bool>,
    closed: Cell<bool>,
}
impl Wallet {
    fn refresh(&self, onboarding: &Onboarding) {
        let view = self.view.borrow();
        let busy = view.busy || self.pending.get();
        onboarding.connection_busy.set(busy);
        onboarding.invite.set_sensitive(!busy);
        onboarding.connect_button.set_sensitive(
            !busy
                && trace_commons_contributor::commands::invite_issuer_host(
                    &onboarding.invite.text(),
                )
                .is_some(),
        );
        self.root
            .set_visible(!view.state.is_empty() && view.state != "Unsupported");
        self.commons.set_sensitive(!busy && view.can_edit);
        self.account.set_sensitive(!busy && view.can_edit);
        self.account.set_visible(view.can_start);
        self.start.set_visible(view.can_start);
        self.start
            .set_sensitive(!self.pending.get() && view.can_start);
        self.check
            .set_sensitive(!self.pending.get() && view.can_check);
        self.cancel.set_visible(view.can_cancel);
        if view.tone == "refused" {
            self.message.add_css_class("tc-refused");
        } else {
            self.message.remove_css_class("tc-refused");
        }
        self.message.set_label(&format!(
            "{}{}{}",
            view.glyph,
            if view.glyph.is_empty() { "" } else { " " },
            view.message
        ));
    }
}
fn widgets() -> Rc<Wallet> {
    let copy = witness_copy().wallet;
    let wallet = Rc::new(Wallet {
        root: gtk::Box::new(gtk::Orientation::Vertical, 12),
        commons: gtk::Entry::builder().placeholder_text(copy.commons).build(),
        account: gtk::Entry::builder().placeholder_text(copy.account).build(),
        check: gtk::Button::with_label(copy.check),
        start: gtk::Button::with_label(copy.start),
        cancel: gtk::Button::with_label(copy.cancel),
        message: body_label(""),
        view: RefCell::new(View::default()),
        pending: Cell::new(false),
        closed: Cell::new(false),
    });
    wallet.root.set_visible(false);
    wallet.root.append(&body_label(copy.heading));
    wallet.root.append(&body_label(copy.disclosure));
    for widget in [
        &wallet.commons.clone().upcast::<gtk::Widget>(),
        &wallet.check.clone().upcast(),
        &wallet.account.clone().upcast(),
        &wallet.start.clone().upcast(),
        &wallet.message.clone().upcast(),
        &wallet.cancel.clone().upcast(),
    ] {
        wallet.root.append(widget);
    }
    wallet
}
pub(super) fn build(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let wallet = widgets();
    for (button, action) in [
        (&wallet.check, "check"),
        (&wallet.start, "start"),
        (&wallet.cancel, "cancel"),
    ] {
        let wallet = wallet.clone();
        let app = app.clone();
        let onboarding = onboarding.clone();
        button.connect_clicked(move |_| {
            if action != "cancel" && (onboarding.connection_busy.get() || wallet.closed.get()) {
                return;
            }
            command(&app, &onboarding, &wallet, action);
        });
    }
    onboarding.window.connect_close_request({
        let app = app.clone();
        let onboarding = onboarding.clone();
        let wallet = wallet.clone();
        move |_| {
            wallet.closed.set(true);
            command(&app, &onboarding, &wallet, "cancel");
            glib::Propagation::Proceed
        }
    });
    command(app, onboarding, &wallet, "open");
    wallet.root.clone()
}
fn command(app: &Rc<App>, onboarding: &Rc<Onboarding>, wallet: &Rc<Wallet>, action: &'static str) {
    wallet.pending.set(true);
    wallet.refresh(onboarding);
    let params = serde_json::json!({"action":action,"flow_id":wallet.view.borrow().flow_id,"ingest_url":wallet.commons.text().as_str(),"account_id":wallet.account.text().as_str()});
    app.call("native_wallet_flow", params, {
        let wallet = wallet.clone();
        let onboarding = onboarding.clone();
        move |app, result| {
            wallet.pending.set(false);
            match result
                .ok()
                .and_then(|v| serde_json::from_value::<View>(v).ok())
            {
                Some(view) => *wallet.view.borrow_mut() = view,
                None => {
                    let copy = witness_copy().wallet;
                    let mut view = wallet.view.borrow_mut();
                    view.message = copy.failed.into();
                    view.tone = copy.refused_tone.into();
                    view.glyph = copy.refused_glyph.into();
                    drop(view);
                    wallet.refresh(&onboarding);
                    return;
                }
            }
            wallet.refresh(&onboarding);
            if wallet.closed.get() {
                if action != "cancel" {
                    command(app, &onboarding, &wallet, "cancel");
                }
                return;
            }
            if action == "start" {
                let browser = wallet.view.borrow().browser_url.clone();
                if let Some(browser) = browser {
                    if gtk::gio::AppInfo::launch_default_for_uri(
                        &browser,
                        None::<&gtk::gio::AppLaunchContext>,
                    )
                    .is_err()
                    {
                        command(app, &onboarding, &wallet, "cancel");
                        return;
                    }
                }
            }
            let wait = wallet.view.borrow().wait;
            if wait {
                command(app, &onboarding, &wallet, "wait");
            } else if wallet.view.borrow().state == "Complete" {
                wallet.account.set_text("");
                onboarding.invite.set_text("");
                load_consent_options(app, &onboarding);
                onboarding.go(Step::Consent);
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wallet_adapter_reads_core_capabilities_without_inventing_them() {
        let empty: View = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!empty.can_start && !empty.wait);
        let view:View=serde_json::from_value(serde_json::json!({"state":"Refused","can_cancel":true,"glyph":"⊘","message":"fixture","tone":"refused"})).unwrap();
        assert!(view.can_cancel);
        assert!(!view.can_start);
        assert_eq!(view.glyph, "⊘");
    }
    /// Only synthetic widget data; never constructs App, a daemon, or a wallet request.
    #[test]
    #[ignore = "requires a display; run under Xvfb with --ignored --test-threads=1"]
    fn wallet_widget_render() {
        gtk::init().expect("display");
        super::super::style::install();
        super::super::community_brand::install();
        let wallet = widgets();
        wallet.root.set_visible(true);
        wallet.commons.set_text("https://commons.example");
        wallet.account.set_text("synthetic.near");
        wallet.cancel.set_visible(false);
        wallet
            .message
            .set_label("This commons supports wallet signup.");
        wallet.root.set_margin_top(24);
        wallet.root.set_margin_bottom(24);
        wallet.root.set_margin_start(24);
        wallet.root.set_margin_end(24);
        let window = gtk::Window::builder()
            .default_width(520)
            .default_height(550)
            .child(&wallet.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        let end = std::time::Instant::now() + std::time::Duration::from_millis(400);
        while std::time::Instant::now() < end {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(wallet.start.is_mapped());
        assert!(wallet.start.allocation().y() + wallet.start.height() <= wallet.root.height());
        if let Ok(path) = std::env::var("TC_WALLET_RENDER_PATH") {
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            let node = snapshot.to_node().expect("render node");
            let renderer = gtk::gsk::Renderer::for_surface(&window.surface()).expect("renderer");
            renderer
                .render_texture(&node, None)
                .save_to_png(path)
                .expect("PNG");
            renderer.unrealize();
        }
        window.close();
    }
}
