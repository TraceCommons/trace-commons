//! Home's hero cards: when each shows and what closes it.
//!
//! A port of the Mac's `HomeClosedCards` and `PrivateAICampaign`
//! (`macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift`,
//! `macos/Sources/TCShellCore/PrivateAICampaign.swift`):
//!
//! - The campaign card shows while promotions are on (Settings' switch, on
//!   by default) and it is not the campaign the person closed with its X,
//!   as it was then. A closed campaign comes back when anything it shows
//!   changes: the fingerprint is every field it draws.
//! - The Missions card shows while missions are on; the Insights card while
//!   the core's Insights heading has loaded. Neither has an X.
//!
//! What Home remembers it keeps in one small JSON file of its own, beside
//! `monitor-window.json` (`monitor::prefs`): the closed campaign, the
//! promotions switch, and the Balance figure last shown, which the Mac
//! keeps in its defaults. Nothing here is the daemon's and nothing leaves
//! the machine.
//!
//! The campaign is the one compiled into the core
//! (`preview_copy::private_ai_campaign_json`), which the Mac also shows
//! until and unless its feed answers. This shell does not read the feed.

use crate::ui::hero_art::arrival::Reading;

/// The symbols a campaign may name (`PrivateAICampaign`).
pub const CAMPAIGN_SYMBOLS: [&str; 4] = ["cpu", "sparkles", "bolt", "globe"];

/// A small, plain-text campaign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Campaign {
    pub id: String,
    pub title: String,
    pub body: String,
    pub action_title: String,
    pub action_url: String,
    pub symbol: String,
    pub image_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct Wire {
    schema_version: i64,
    id: String,
    title: String,
    body: String,
    action_title: String,
    action_url: String,
    symbol: String,
    #[serde(default)]
    image_url: Option<String>,
}

/// Plain text: not blank, within `limit` characters, no control
/// characters, no angle brackets.
fn plain(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= limit
        && !value.chars().any(char::is_control)
        && !value.contains('<')
        && !value.contains('>')
}

/// An https URL on near.ai with no credentials, no other port and no
/// fragment.
fn near_ai_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    if url.contains('#') {
        return false;
    }
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, "443")) => host,
        Some(_) => return false,
        None => authority,
    };
    host.eq_ignore_ascii_case("near.ai")
}

impl Campaign {
    /// The campaign, or none when it is not one the Mac would show.
    ///
    /// The Mac also accepts an `image_url` beside its feed; this shell
    /// reads no feed and draws no remote image, so a campaign with one is
    /// drawn with its symbol.
    pub fn decode(json: &str) -> Option<Self> {
        if json.len() > 16 * 1024 {
            return None;
        }
        let wire: Wire = serde_json::from_str(json).ok()?;
        let valid = wire.schema_version == 1
            && plain(&wire.id, 100)
            && plain(&wire.title, 100)
            && plain(&wire.body, 300)
            && plain(&wire.action_title, 40)
            && CAMPAIGN_SYMBOLS.contains(&wire.symbol.as_str())
            && near_ai_url(&wire.action_url);
        valid.then_some(Self {
            id: wire.id,
            title: wire.title,
            body: wire.body,
            action_title: wire.action_title,
            action_url: wire.action_url,
            symbol: wire.symbol,
            image_url: wire.image_url,
        })
    }

    /// The campaign compiled into the core.
    pub fn compiled() -> Option<Self> {
        Self::decode(trace_commons_contributor::preview_copy::private_ai_campaign_json())
    }

    /// Everything the card shows, so any change is a new fingerprint
    /// (`HomeClosedCards.fingerprint`).
    pub fn fingerprint(&self) -> String {
        [
            self.id.as_str(),
            &self.title,
            &self.body,
            &self.action_title,
            &self.action_url,
            &self.symbol,
            self.image_url.as_deref().unwrap_or(""),
        ]
        .join("\u{1F}")
    }
}

/// A campaign shows while promotions are on and it is not the one the
/// person closed, as it was then (`HomeClosedCards.showsCampaign`).
pub fn shows_campaign(fingerprint: &str, closed: &str, promotions_on: bool) -> bool {
    promotions_on && fingerprint != closed
}

/// The Missions hero's line (`HomeOverview.drafts`): the newest draft and
/// how many more, the core's empty line, or a dash before the inbox has
/// answered.
pub fn drafts_line(ids: Option<&[String]>, empty: &str) -> String {
    match ids {
        None => "—".to_owned(),
        Some([]) => empty.to_owned(),
        Some([first]) => first.clone(),
        Some([first, rest @ ..]) => format!("{first} · +{}", rest.len()),
    }
}

/// What Home remembers across launches.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HomePrefs {
    /// The fingerprint of the campaign as it was when closed.
    pub closed_campaign: String,
    /// Settings' "Show promotions", on by default.
    pub shows_promotions: bool,
    /// The Balance figure and received total last shown, so credit that
    /// arrived while the app was closed is marked the next time.
    pub balance_shown: Option<Reading>,
}

impl Default for HomePrefs {
    fn default() -> Self {
        Self {
            closed_campaign: String::new(),
            shows_promotions: true,
            balance_shown: None,
        }
    }
}

impl HomePrefs {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }

    pub fn load() -> Self {
        file()
            .and_then(|path| std::fs::read(path).ok())
            .map(|bytes| Self::from_bytes(&bytes))
            .unwrap_or_default()
    }

    /// A failed write is dropped: it can never keep Home from drawing.
    pub fn save(&self) {
        let Some(path) = file() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

#[cfg(test)]
thread_local! {
    static TEST_FILE: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_file() -> Option<std::path::PathBuf> {
    TEST_FILE.with(|file| file.borrow().clone())
}

#[cfg(not(test))]
fn test_file() -> Option<std::path::PathBuf> {
    None
}

/// A test's own file for this thread; none reads and writes nothing.
#[cfg(test)]
pub fn set_test_file(path: Option<std::path::PathBuf>) {
    TEST_FILE.with(|file| *file.borrow_mut() = path);
}

/// `TC_HOME_STATE` names the file for a test or a capture run; a test build
/// otherwise reads and writes nothing.
fn file() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("TC_HOME_STATE") {
        return Some(std::path::PathBuf::from(path));
    }
    // A test build never reads or writes the person's own file.
    if cfg!(test) {
        return test_file();
    }
    crate::shell_state_file("home.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campaign(edit: impl FnOnce(&mut serde_json::Value)) -> Option<Campaign> {
        let mut value: serde_json::Value = serde_json::from_str(
            trace_commons_contributor::preview_copy::private_ai_campaign_json(),
        )
        .unwrap();
        edit(&mut value);
        Campaign::decode(&value.to_string())
    }

    #[test]
    fn the_compiled_campaign_decodes_and_names_a_drawn_symbol() {
        let compiled = Campaign::compiled().expect("the core's campaign is valid");
        assert!(CAMPAIGN_SYMBOLS.contains(&compiled.symbol.as_str()));
        assert!(crate::ui::hero_art::glyph::hero_mask(&compiled.symbol).is_some());
        for symbol in CAMPAIGN_SYMBOLS {
            assert!(
                crate::ui::hero_art::glyph::hero_mask(symbol).is_some(),
                "{symbol}"
            );
        }
    }

    #[test]
    fn a_campaign_the_mac_would_refuse_is_not_drawn() {
        assert!(campaign(|v| v["schema_version"] = 2.into()).is_none());
        assert!(campaign(|v| v["symbol"] = "person".into()).is_none());
        assert!(campaign(|v| v["title"] = " ".into()).is_none());
        assert!(campaign(|v| v["title"] = "x".repeat(101).into()).is_none());
        assert!(campaign(|v| v["body"] = "<b>bold</b>".into()).is_none());
        assert!(campaign(|v| v["action_title"] = "a\nb".into()).is_none());
        for url in [
            "http://near.ai/red",
            "https://evil.example/red",
            "https://near.ai.evil.example/",
            "https://user@near.ai/",
            "https://near.ai:8443/",
            "https://near.ai/red#top",
        ] {
            assert!(
                campaign(|v| v["action_url"] = url.into()).is_none(),
                "{url}"
            );
        }
        assert!(campaign(|v| v["action_url"] = "https://NEAR.ai:443/red?x=1".into()).is_some());
    }

    #[test]
    fn a_closed_campaign_comes_back_when_what_it_shows_changes() {
        let shown = Campaign::compiled().unwrap();
        let closed = shown.fingerprint();
        assert!(!shows_campaign(&shown.fingerprint(), &closed, true));
        let reworded = campaign(|v| v["body"] = "Something new.".into()).unwrap();
        assert!(shows_campaign(&reworded.fingerprint(), &closed, true));
        // Promotions off hides every campaign.
        assert!(!shows_campaign(&reworded.fingerprint(), "", false));
        assert!(shows_campaign(&shown.fingerprint(), "", true));
        assert_eq!(shown.fingerprint().matches('\u{1F}').count(), 6);
    }

    #[test]
    fn the_missions_line_never_reads_an_unanswered_inbox_as_empty() {
        assert_eq!(drafts_line(None, "None."), "—");
        assert_eq!(drafts_line(Some(&[]), "None."), "None.");
        assert_eq!(drafts_line(Some(&["a".into()]), "None."), "a");
        assert_eq!(
            drafts_line(Some(&["a".into(), "b".into(), "c".into()]), "None."),
            "a · +2"
        );
    }

    #[test]
    fn prefs_default_to_promotions_on_and_survive_older_files() {
        assert_eq!(HomePrefs::from_bytes(b""), HomePrefs::default());
        assert!(HomePrefs::default().shows_promotions);
        let older = HomePrefs::from_bytes(br#"{"closed_campaign":"x"}"#);
        assert_eq!(older.closed_campaign, "x");
        assert!(older.shows_promotions);
        let chosen = HomePrefs {
            closed_campaign: "fp".into(),
            shows_promotions: false,
            balance_shown: Some(Reading {
                received_nanos: Some(3),
                remaining_nanos: None,
            }),
        };
        let bytes = serde_json::to_vec(&chosen).unwrap();
        assert_eq!(HomePrefs::from_bytes(&bytes), chosen);
    }
}
