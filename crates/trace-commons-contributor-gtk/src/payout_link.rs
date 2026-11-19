//! The NEAR AI payout section's decisions, as the Mac makes them
//! (`PayoutLinkSurface`, `PayoutPicker` in `PayoutLinkSurface.swift`): which
//! organization the commons sends NEAR AI credit to. Pure: every sentence is
//! the core's (`consent_copy::payout_link_copy`) and every fact the
//! daemon's (`payout_rails_status`, `payout_link_orgs`). The GTK view is
//! `ui::payout_link`.
use serde::Deserialize;
use trace_commons_contributor::consent_copy::PayoutLinkCopy;

/// The rail this section is for.
pub const NEAR_AI_RAIL: &str = "near_ai";

/// One rail in `payout_rails_status`. Strings are the daemon's own labels.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Rail {
    pub rail_id: String,
    pub mode: String,
    pub linked: bool,
    #[serde(default)]
    pub state: Option<String>,
    pub version: i64,
    #[serde(default)]
    pub request_state: Option<String>,
    #[serde(default)]
    pub unlink_requested: bool,
    #[serde(default)]
    pub recipient_unavailable: bool,
    /// `match`, `mismatch` or `unknown`; None when the commons reports none.
    #[serde(default)]
    pub recipient_ref_check: Option<String>,
    #[serde(default)]
    pub organization_name: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
}

/// An organization the signed-in NEAR AI user owns and may choose.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Organization {
    pub id: String,
    pub name: String,
}

/// `payout_link_orgs`: the owned orgs and the revision a start must echo.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Listing {
    pub organizations: Vec<Organization>,
    pub orgs_revision: String,
}

/// Why a read, start or confirm did not go through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No NEAR AI session, or it ended.
    SignIn,
    /// No Trace Commons account session.
    SessionEnded,
    /// The org list or the link changed after it was shown.
    Changed,
    /// Anything else; nothing was saved.
    Failed,
}

impl Refusal {
    /// The daemon's refusal label, routed (`PayoutLinkRefusal.from`).
    pub fn from_label(label: &str) -> Self {
        match label {
            "payout-link-near-ai-sign-in-required"
            | "payout-link-near-ai-session-expired"
            | "payout-link-near-ai-session-changed" => Self::SignIn,
            "account-session-required" => Self::SessionEnded,
            "payout-link-orgs-changed"
            | "payout-link-changed"
            | "version_conflict"
            | "org_not_owned"
            | "payout_link_change_pending" => Self::Changed,
            _ => Self::Failed,
        }
    }

    /// The line for this refusal.
    pub fn line(self, copy: &PayoutLinkCopy) -> &'static str {
        match self {
            Self::SignIn => copy.sign_in,
            Self::SessionEnded => copy.session_ended,
            Self::Changed => copy.changed,
            Self::Failed => copy.failed,
        }
    }
}

/// The rail when the commons enables it (`dry_run` or `live`).
pub fn near_ai_rail(status: &serde_json::Value) -> Option<Rail> {
    status["rails"]
        .as_array()?
        .iter()
        .filter_map(|rail| serde_json::from_value::<Rail>(rail.clone()).ok())
        .find(|rail| {
            rail.rail_id == NEAR_AI_RAIL && matches!(rail.mode.as_str(), "live" | "dry_run")
        })
}

fn named(template: &str, rail: &Rail) -> Option<String> {
    rail.organization_name
        .as_deref()
        .map(|name| template.replace("{org_name}", name))
}

/// The status line, or None while it needs a name only the org list can
/// supply. A mismatch is never hidden behind a friendlier state.
pub fn status_line(copy: &PayoutLinkCopy, rail: &Rail) -> Option<String> {
    if rail.recipient_ref_check.as_deref() == Some("mismatch") {
        return Some(copy.status_mismatch.to_owned());
    }
    if rail.recipient_unavailable {
        return Some(copy.status_unavailable.to_owned());
    }
    if !rail.linked || rail.unlink_requested {
        return Some(copy.status_none.to_owned());
    }
    match rail.state.as_deref() {
        Some("active") => named(copy.status_linked, rail),
        Some("waiting") => named(copy.status_waiting, rail),
        // A closing link, or a state this build does not know: credit is
        // not promised anywhere.
        _ => Some(copy.status_none.to_owned()),
    }
}

/// Whether the person may choose an organization now.
pub fn can_choose(rail: &Rail) -> bool {
    if rail.recipient_ref_check.as_deref() == Some("mismatch") || rail.recipient_unavailable {
        return true;
    }
    !rail.linked || rail.unlink_requested
}

/// Whether stop is offered: an active link nothing is closing, whose name
/// is known.
pub fn can_unlink(rail: &Rail) -> bool {
    rail.linked
        && !rail.unlink_requested
        && rail.state.as_deref() == Some("active")
        && rail.organization_name.is_some()
}

/// Whether the direct change is offered: an active link nothing is closing.
pub fn can_change(rail: &Rail) -> bool {
    rail.linked
        && !rail.unlink_requested
        && rail.state.as_deref() == Some("active")
        && rail.recipient_ref_check.as_deref() != Some("mismatch")
        && !rail.recipient_unavailable
}

/// Where the picker stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Loading,
    /// The owned orgs, none selected until the person picks one.
    Choosing,
    /// The person owns no active org.
    NoOrganization,
    /// A start and confirm are in flight.
    Working,
    Refused(Refusal),
}

/// The picker. Nothing is ever preselected, however many orgs there are,
/// and choosing needs the person's click after the consent is shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picker {
    pub phase: Phase,
    pub listing: Option<Listing>,
    pub selected: Option<String>,
    /// Change pressed on an active link.
    pub changing: bool,
}

/// The consent for the selected org, shown before Choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    pub body: String,
    pub scope: String,
    pub accessibility_name: String,
}

impl Picker {
    pub fn rows(&self) -> &[Organization] {
        self.listing
            .as_ref()
            .map_or(&[][..], |l| l.organizations.as_slice())
    }
    pub fn selected_org(&self) -> Option<&Organization> {
        let id = self.selected.as_deref()?;
        self.rows().iter().find(|o| o.id == id)
    }
    pub fn begin_loading(&mut self) {
        self.phase = Phase::Loading;
    }
    /// Change on an active link: list, then the same consent and Choose.
    pub fn begin_change(&mut self) {
        self.changing = true;
        self.selected = None;
        self.phase = Phase::Loading;
    }
    /// A fresh list replaces the old one and clears any selection.
    pub fn listed(&mut self, listing: Listing) {
        self.phase = if listing.organizations.is_empty() {
            Phase::NoOrganization
        } else {
            Phase::Choosing
        };
        self.listing = Some(listing);
        self.selected = None;
    }
    pub fn select(&mut self, id: &str) {
        if self.phase == Phase::Choosing && self.rows().iter().any(|o| o.id == id) {
            self.selected = Some(id.to_owned());
        }
    }
    /// "Not now": back to the list, or out of a change.
    pub fn dismiss_selection(&mut self) {
        self.selected = None;
        if self.changing {
            self.changing = false;
            self.listing = None;
            self.phase = Phase::Idle;
            return;
        }
        if matches!(
            self.phase,
            Phase::Choosing | Phase::Refused(Refusal::Failed)
        ) {
            self.phase = if self.rows().is_empty() {
                Phase::Idle
            } else {
                Phase::Choosing
            };
        }
    }
    pub fn begin_working(&mut self) {
        if self.selected_org().is_some() && self.phase == Phase::Choosing {
            self.phase = Phase::Working;
        }
    }
    /// A refusal other than a plain failure drops the list: its rows are
    /// not to be trusted, and a retry lists again.
    pub fn refused(&mut self, refusal: Refusal) {
        self.selected = None;
        if refusal != Refusal::Failed {
            self.listing = None;
            self.changing = false;
        }
        self.phase = Phase::Refused(refusal);
    }
    pub fn finished(&mut self) {
        *self = Self::default();
    }
    pub fn consent(&self, copy: &PayoutLinkCopy) -> Option<Consent> {
        let org = self.selected_org()?;
        if !matches!(self.phase, Phase::Choosing | Phase::Working) {
            return None;
        }
        let fill = |template: &str| template.replace("{org_name}", &org.name);
        Some(Consent {
            body: fill(copy.body),
            scope: fill(copy.scope),
            accessibility_name: fill(copy.accessibility_name),
        })
    }
}

/// Whether Change is drawn: on an active link, except while the change's
/// list or consent is on screen.
pub fn shows_change_button(rail: &Rail, picker: &Picker) -> bool {
    if !can_change(rail) {
        return false;
    }
    !picker.changing
        || matches!(
            picker.phase,
            Phase::Idle | Phase::NoOrganization | Phase::Refused(_)
        )
}

/// Whether the org list and consent are drawn.
pub fn shows_picker(rail: &Rail, picker: &Picker) -> bool {
    can_choose(rail) || picker.changing
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use trace_commons_contributor::consent_copy::payout_link_copy;

    fn rail(extra: serde_json::Value) -> Rail {
        let mut value = json!({"rail_id":"near_ai","mode":"live","linked":false,"version":0});
        for (k, v) in extra.as_object().unwrap() {
            value[k] = v.clone();
        }
        serde_json::from_value(value).unwrap()
    }
    fn active(name: Option<&str>) -> Rail {
        rail(
            json!({"linked":true,"state":"active","recipient_ref_check":"match",
            "organization_id":"o1","organization_name":name}),
        )
    }
    fn listing(ids: &[&str]) -> Listing {
        Listing {
            organizations: ids
                .iter()
                .map(|id| Organization {
                    id: (*id).into(),
                    name: format!("Org {id}"),
                })
                .collect(),
            orgs_revision: "r1".into(),
        }
    }

    #[test]
    fn only_an_enabled_near_ai_rail_is_shown() {
        let status = |mode: &str, id: &str| json!({"rails":[{"rail_id":id,"mode":mode,"linked":false,"version":0}]});
        assert!(near_ai_rail(&status("live", "near_ai")).is_some());
        assert!(near_ai_rail(&status("dry_run", "near_ai")).is_some());
        assert!(near_ai_rail(&status("disabled", "near_ai")).is_none());
        assert!(near_ai_rail(&status("live", "other")).is_none());
        assert!(near_ai_rail(&json!({})).is_none());
    }

    #[test]
    fn the_status_line_never_hides_a_mismatch() {
        let copy = payout_link_copy();
        assert_eq!(
            status_line(
                &copy,
                &rail(json!({"recipient_ref_check":"mismatch","linked":true,"state":"active"}))
            )
            .as_deref(),
            Some(copy.status_mismatch)
        );
        assert_eq!(
            status_line(&copy, &rail(json!({"recipient_unavailable":true}))).as_deref(),
            Some(copy.status_unavailable)
        );
        assert_eq!(
            status_line(&copy, &rail(json!({}))).as_deref(),
            Some(copy.status_none)
        );
        assert_eq!(
            status_line(&copy, &active(Some("Acme"))),
            Some(copy.status_linked.replace("{org_name}", "Acme"))
        );
        assert_eq!(
            status_line(&copy, &active(None)),
            None,
            "needs the org list's name"
        );
        let waiting = rail(json!({"linked":true,"state":"waiting","organization_name":"Acme"}));
        assert_eq!(
            status_line(&copy, &waiting),
            Some(copy.status_waiting.replace("{org_name}", "Acme"))
        );
        let closing = rail(json!({"linked":true,"state":"closing","organization_name":"Acme"}));
        assert_eq!(
            status_line(&copy, &closing).as_deref(),
            Some(copy.status_none)
        );
    }

    #[test]
    fn choose_change_and_stop_follow_the_link() {
        let free = rail(json!({}));
        assert!(can_choose(&free));
        assert!(!can_change(&free));
        assert!(!can_unlink(&free));
        let linked = active(Some("Acme"));
        assert!(!can_choose(&linked));
        assert!(can_change(&linked));
        assert!(can_unlink(&linked));
        assert!(!can_unlink(&active(None)), "the stop button names the org");
        let mismatch = rail(
            json!({"linked":true,"state":"active","recipient_ref_check":"mismatch","organization_name":"Acme"}),
        );
        assert!(can_choose(&mismatch));
        assert!(!can_change(&mismatch));
    }

    #[test]
    fn refusals_route_by_label() {
        let copy = payout_link_copy();
        assert_eq!(
            Refusal::from_label("payout-link-near-ai-session-expired"),
            Refusal::SignIn
        );
        assert_eq!(
            Refusal::from_label("account-session-required"),
            Refusal::SessionEnded
        );
        assert_eq!(Refusal::from_label("version_conflict"), Refusal::Changed);
        assert_eq!(Refusal::from_label("anything"), Refusal::Failed);
        assert_eq!(Refusal::SignIn.line(&copy), copy.sign_in);
        assert_eq!(Refusal::Failed.line(&copy), copy.failed);
    }

    #[test]
    fn nothing_is_preselected_and_consent_comes_before_choose() {
        let copy = payout_link_copy();
        let mut picker = Picker::default();
        picker.begin_loading();
        picker.listed(listing(&["o1"]));
        assert_eq!(picker.phase, Phase::Choosing);
        assert_eq!(picker.selected, None, "even a sole org is not preselected");
        assert_eq!(picker.consent(&copy), None);
        picker.select("nope");
        assert_eq!(picker.selected, None);
        picker.select("o1");
        let consent = picker.consent(&copy).unwrap();
        assert_eq!(consent.body, copy.body.replace("{org_name}", "Org o1"));
        picker.begin_working();
        assert_eq!(picker.phase, Phase::Working);
        picker.finished();
        assert_eq!(picker, Picker::default());
        picker.listed(listing(&[]));
        assert_eq!(picker.phase, Phase::NoOrganization);
    }

    #[test]
    fn a_changed_list_is_dropped_and_change_can_be_left() {
        let mut picker = Picker::default();
        picker.listed(listing(&["o1", "o2"]));
        picker.select("o2");
        picker.refused(Refusal::Changed);
        assert_eq!(picker.listing, None);
        assert_eq!(picker.phase, Phase::Refused(Refusal::Changed));
        let linked = active(Some("Acme"));
        assert!(shows_change_button(&linked, &picker));
        picker.begin_change();
        assert!(picker.changing);
        assert!(
            !shows_change_button(&linked, &picker),
            "loading the change's list"
        );
        assert!(shows_picker(&linked, &picker));
        picker.listed(listing(&["o1"]));
        picker.dismiss_selection();
        assert!(!picker.changing, "Not now leaves the change");
        assert!(!shows_picker(&linked, &picker));
        assert!(shows_change_button(&linked, &picker));
    }
}
