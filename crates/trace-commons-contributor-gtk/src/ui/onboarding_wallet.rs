//! Browser wallet handoff. The daemon alone owns device signing and PKCE.
use super::App;
use super::onboarding::{Onboarding, Step, body_label, load_consent_options};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Wallet {
    root: gtk::Box,
    commons: gtk::Entry,
    account: gtk::Entry,
    check: gtk::Button,
    start: gtk::Button,
    cancel: gtk::Button,
    message: gtk::Label,
    ready: Cell<bool>,
    busy: Cell<bool>,
    closed: Cell<bool>,
    attempt: RefCell<Option<String>>,
}
impl Wallet {
    fn refresh(&self, onboarding: &Onboarding) {
        let busy = self.busy.get() || self.attempt.borrow().is_some();
        onboarding.connection_busy.set(busy);
        onboarding.invite.set_sensitive(!busy);
        onboarding.connect_button.set_sensitive(
            !busy
                && trace_commons_contributor::commands::invite_issuer_host(
                    &onboarding.invite.text(),
                )
                .is_some(),
        );
        self.commons.set_sensitive(!busy);
        self.account.set_sensitive(!busy);
        self.account.set_visible(self.ready.get());
        self.check
            .set_sensitive(!busy && !self.commons.text().trim().is_empty());
        self.start.set_visible(self.ready.get());
        self.start
            .set_sensitive(!busy && !self.account.text().trim().is_empty());
        self.cancel.set_visible(self.attempt.borrow().is_some());
    }
    fn cancel(&self, app: &Rc<App>, onboarding: &Onboarding) {
        if let Some(id) = self.attempt.borrow_mut().take() {
            app.call(
                "near_account_cancel",
                serde_json::json!({"attempt_id":id}),
                |_, _| {},
            );
        }
        self.busy.set(false);
        self.message.set_label("Connection cancelled.");
        self.refresh(onboarding);
    }
}

pub(super) fn supported(value: &serde_json::Value) -> bool {
    value
        .get("methods")
        .and_then(|v| v.as_array())
        .is_some_and(|methods| {
            [
                "near_account_capabilities",
                "near_account_start",
                "near_account_status",
                "near_account_cancel",
            ]
            .iter()
            .all(|required| methods.iter().any(|v| v.as_str() == Some(required)))
        })
}
pub(super) fn browser_destination(commons: &str, browser: &str) -> bool {
    let parse = |value| glib::Uri::parse(value, glib::UriFlags::NONE).ok();
    let (Some(origin), Some(target)) = (parse(commons), parse(browser)) else {
        return false;
    };
    let port = |uri: &glib::Uri| if uri.port() == -1 { 443 } else { uri.port() };
    origin.scheme() == "https"
        && target.scheme() == "https"
        && origin.userinfo().is_none()
        && target.userinfo().is_none()
        && origin.host().is_some()
        && origin.host().as_deref().map(str::to_ascii_lowercase)
            == target.host().as_deref().map(str::to_ascii_lowercase)
        && port(&origin) == port(&target)
}

fn widgets() -> Rc<Wallet> {
    let wallet = Rc::new(Wallet {
        root: gtk::Box::new(gtk::Orientation::Vertical, 12),
        commons: gtk::Entry::builder()
            .placeholder_text("Commons HTTPS address")
            .build(),
        account: gtk::Entry::builder()
            .placeholder_text("Your NEAR account")
            .build(),
        check: gtk::Button::with_label("Check availability"),
        start: gtk::Button::with_label("Continue in wallet"),
        cancel: gtk::Button::with_label("Cancel connection"),
        message: body_label(""),
        ready: Cell::new(false),
        busy: Cell::new(false),
        closed: Cell::new(false),
        attempt: RefCell::new(None),
    });
    wallet.root.set_visible(false);
    wallet.root.append(&body_label("Join with a NEAR account"));
    wallet.root.append(&body_label("Check whether your commons accepts new accounts. Connecting proves control of your account and this device; it does not fund inference or enable capture."));
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
    wallet.refresh(onboarding);
    app.call("hello", serde_json::json!({}), {
        let wallet = wallet.clone();
        move |_, result| {
            wallet
                .root
                .set_visible(result.ok().as_ref().is_some_and(supported))
        }
    });
    wallet.commons.connect_changed({
        let wallet = wallet.clone();
        let onboarding = onboarding.clone();
        move |_| {
            wallet.ready.set(false);
            wallet.refresh(&onboarding);
        }
    });
    wallet.account.connect_changed({
        let wallet = wallet.clone();
        let onboarding = onboarding.clone();
        move |_| wallet.refresh(&onboarding)
    });
    wallet.check.connect_clicked({ let wallet = wallet.clone(); let onboarding = onboarding.clone(); let app = app.clone(); move |_| {
        if onboarding.connection_busy.get() || wallet.closed.get() { return; }
        wallet.busy.set(true); wallet.ready.set(false); wallet.message.set_label(""); wallet.refresh(&onboarding);
        app.call("near_account_capabilities", serde_json::json!({"ingest_url":wallet.commons.text().as_str()}), {
            let wallet = wallet.clone(); let onboarding = onboarding.clone(); move |_, result| {
                wallet.ready.set(result.ok().and_then(|v| v.get("ready").and_then(|b| b.as_bool())).unwrap_or(false));
                wallet.busy.set(false);
                wallet.message.set_label(if wallet.ready.get() { "This commons supports wallet signup." } else { "Wallet signup is unavailable for this commons. You can still use an invite." });
                wallet.refresh(&onboarding);
            }
        });
    }});
    wallet.start.connect_clicked({ let wallet = wallet.clone(); let onboarding = onboarding.clone(); let app = app.clone(); move |_| {
        if !wallet.ready.get() || onboarding.connection_busy.get() || wallet.closed.get() || wallet.account.text().trim().is_empty() { return; }
        wallet.busy.set(true); wallet.message.set_label("Opening a wallet connection…"); wallet.refresh(&onboarding);
        app.call("near_account_start", serde_json::json!({"ingest_url":wallet.commons.text().as_str(), "account_id":wallet.account.text().trim()}), {
            let wallet = wallet.clone(); let onboarding = onboarding.clone(); move |app, result| {
                let value = result.ok().unwrap_or_default();
                let id = value.get("attempt_id").and_then(|v| v.as_str()).filter(|id| !id.is_empty());
                *wallet.attempt.borrow_mut() = id.map(str::to_owned);
                let browser = value.get("browser_url").and_then(|v| v.as_str()).unwrap_or("");
                if wallet.closed.get() || id.is_none() || value.get("status").and_then(|v| v.as_str()) != Some("waiting_for_wallet") || !browser_destination(&wallet.commons.text(), browser) {
                    wallet.cancel(app, &onboarding); wallet.message.set_label("The connection could not start. Check availability and try again."); return;
                }
                if gtk::gio::AppInfo::launch_default_for_uri(browser, None::<&gtk::gio::AppLaunchContext>).is_err() { wallet.cancel(app, &onboarding); return; }
                wallet.busy.set(false); wallet.message.set_label("Finish signing in your wallet. Keep this window open."); wallet.refresh(&onboarding);
                poll(app, &onboarding, &wallet, id.unwrap().to_owned());
            }
        });
    }});
    wallet.cancel.connect_clicked({
        let wallet = wallet.clone();
        let onboarding = onboarding.clone();
        let app = app.clone();
        move |_| wallet.cancel(&app, &onboarding)
    });
    onboarding.window.connect_close_request({
        let wallet = wallet.clone();
        let onboarding = onboarding.clone();
        let app = app.clone();
        move |_| {
            wallet.closed.set(true);
            wallet.cancel(&app, &onboarding);
            glib::Propagation::Proceed
        }
    });
    wallet.root.clone()
}
fn poll(app: &Rc<App>, onboarding: &Rc<Onboarding>, wallet: &Rc<Wallet>, id: String) {
    if wallet.closed.get() || wallet.attempt.borrow().as_deref() != Some(&id) {
        return;
    }
    app.call(
        "near_account_status",
        serde_json::json!({"attempt_id":id}),
        {
            let wallet = wallet.clone();
            let onboarding = onboarding.clone();
            move |app, result| {
                if wallet.closed.get() || wallet.attempt.borrow().as_deref() != Some(&id) {
                    return;
                }
                let value = result.ok().unwrap_or_default();
                match value.get("status").and_then(|v| v.as_str()) {
                    Some("complete") => {
                        wallet.attempt.borrow_mut().take();
                        wallet.account.set_text("");
                        wallet.refresh(&onboarding);
                        onboarding.invite.set_text("");
                        load_consent_options(app, &onboarding);
                        onboarding.go(Step::Consent);
                    }
                    Some("failed" | "cancelled" | "expired") => {
                        wallet.attempt.borrow_mut().take();
                        wallet.message.set_label(
                            "The wallet connection did not complete. You can try again.",
                        );
                        wallet.refresh(&onboarding);
                    }
                    Some("starting" | "waiting_for_wallet") => {
                        let app = app.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_secs(2),
                            move || poll(&app, &onboarding, &wallet, id),
                        );
                    }
                    _ => wallet
                        .message
                        .set_label("The connection status is unavailable. Cancel and try again."),
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn wallet_browser_requires_exact_https_origin() {
        for bad in [
            "http://commons.example/wallet",
            "https://elsewhere.example/wallet",
            "https://commons.example:444/wallet",
            "https://user@commons.example/wallet",
            "file:///tmp/wallet",
        ] {
            assert!(!browser_destination("https://commons.example", bad));
        }
        assert!(browser_destination(
            "https://commons.example",
            "https://commons.example:443/wallet?ceremony=synthetic"
        ));
    }
    #[test]
    fn wallet_needs_all_lifecycle_methods() {
        assert!(!supported(
            &serde_json::json!({"methods":["near_account_start"]})
        ));
        assert!(!supported(&serde_json::json!({})));
        assert!(supported(
            &serde_json::json!({"methods":["near_account_capabilities","near_account_start","near_account_status","near_account_cancel"]})
        ));
    }
}
