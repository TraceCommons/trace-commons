//! Single-organization setup after sign-in, as the Mac prepares it
//! (`SingleOrganizationOnboarding.prepare`): with exactly one cloud
//! organization, select it for spending and link it for payouts, then Start
//! runs by itself. Every other answer leaves Join to the person. Pure: each
//! function reads one daemon answer and says what comes next; the GTK
//! coordinator makes the calls in order and stops at the first `Err`.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ready,
    NeedsChoice,
    Unavailable,
}

/// What the rail answered: already linked to the one organization, or free
/// to link at `version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    AlreadyLinked,
    Link { version: i64 },
}

pub const NEAR_AI_RAIL: &str = "near_ai";

/// `near_ai_spend_organizations` (fresh): the one organization's id.
pub fn after_cloud(cloud: &Value) -> Result<String, Outcome> {
    let ids = listed(cloud)?;
    match ids.as_slice() {
        [id] => Ok(id.clone()),
        _ => Err(Outcome::NeedsChoice),
    }
}

/// The listed organizations' ids, as `SpendOrganizations.parse` reads them:
/// any malformed row is unreported, and only `listed` with no revoke
/// pending can be acted on.
fn listed(cloud: &Value) -> Result<Vec<String>, Outcome> {
    if cloud["state"].as_str() != Some("listed") || cloud["revoke_pending"].as_bool() == Some(true)
    {
        return Err(Outcome::Unavailable);
    }
    let rows = cloud["organizations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    rows.iter()
        .map(|row| {
            let id = row["id"].as_str().filter(|s| !s.is_empty());
            let name = row["name"].as_str().filter(|s| !s.is_empty());
            match (id, name) {
                (Some(id), Some(_)) => Ok(id.to_owned()),
                _ => Err(Outcome::Unavailable),
            }
        })
        .collect()
}

/// `payout_link_orgs`: the revision to echo, when it owns exactly that one.
pub fn after_owned(owned: &Value, id: &str) -> Result<String, Outcome> {
    let rows = owned["organizations"]
        .as_array()
        .ok_or(Outcome::Unavailable)?;
    let revision = owned["orgs_revision"]
        .as_str()
        .ok_or(Outcome::Unavailable)?;
    if rows.len() != 1 || rows[0]["id"].as_str() != Some(id) || revision.is_empty() {
        return Err(Outcome::NeedsChoice);
    }
    Ok(revision.to_owned())
}

/// `payout_rails_status`: never silently redirect another recipient.
pub fn after_rails(status: &Value, id: &str) -> Result<Rail, Outcome> {
    let rail = near_ai_rail(status).ok_or(Outcome::Unavailable)?;
    if matches(rail, id) {
        return Ok(Rail::AlreadyLinked);
    }
    let free = rail["linked"].as_bool() == Some(false)
        && rail["state"].is_null()
        && rail["version"].as_i64() == Some(0)
        && rail["request_state"].is_null()
        && rail["unlink_requested"].as_bool() != Some(true)
        && rail["recipient_unavailable"].as_bool() != Some(true)
        && rail["recipient_ref_check"].is_null();
    let version = rail["version"].as_i64().ok_or(Outcome::Unavailable)?;
    if free {
        Ok(Rail::Link { version })
    } else {
        Err(Outcome::NeedsChoice)
    }
}

fn near_ai_rail(status: &Value) -> Option<&Value> {
    status["rails"].as_array()?.iter().find(|rail| {
        rail["rail_id"].as_str() == Some(NEAR_AI_RAIL)
            && matches!(rail["mode"].as_str(), Some("live" | "dry_run"))
    })
}

fn matches(rail: &Value, id: &str) -> bool {
    rail["linked"].as_bool() == Some(true)
        && rail["state"].as_str() == Some("active")
        && rail["recipient_ref_check"].as_str() == Some("match")
        && rail["organization_id"].as_str() == Some(id)
        && rail["request_state"].is_null()
        && rail["unlink_requested"].as_bool() != Some(true)
        && rail["recipient_unavailable"].as_bool() != Some(true)
}

/// `near_ai_spend_organizations` after the select: it is the current one.
pub fn after_selected(selected: &Value, id: &str) -> Result<(), Outcome> {
    let ids = listed(selected)?;
    if ids == [id] && selected["current_organization_id"].as_str() == Some(id) {
        Ok(())
    } else {
        Err(Outcome::Unavailable)
    }
}

/// The final `payout_rails_status`: linked, active and matching.
pub fn confirmed(status: &Value, id: &str) -> Outcome {
    if near_ai_rail(status).is_some_and(|rail| matches(rail, id)) {
        Outcome::Ready
    } else {
        Outcome::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cloud(ids: &[&str]) -> Value {
        json!({
            "state": "listed",
            "organizations": ids.iter().map(|id| json!({"id":id,"name":"Org","current":false})).collect::<Vec<_>>(),
            "current_organization_id": null,
            "revoke_pending": false
        })
    }
    fn rail(extra: Value) -> Value {
        let mut rail = json!({"rail_id":"near_ai","mode":"live","linked":false,"version":0});
        for (k, v) in extra.as_object().unwrap() {
            rail[k] = v.clone();
        }
        json!({"rails":[rail]})
    }
    fn linked(id: &str) -> Value {
        rail(
            json!({"linked":true,"state":"active","recipient_ref_check":"match","organization_id":id}),
        )
    }

    #[test]
    fn one_cloud_organization_is_the_only_way_to_ready() {
        assert_eq!(after_cloud(&cloud(&["o1"])), Ok("o1".into()));
        assert_eq!(
            after_cloud(&cloud(&["o1", "o2"])),
            Err(Outcome::NeedsChoice)
        );
        assert_eq!(after_cloud(&cloud(&[])), Err(Outcome::NeedsChoice));
        let mut pending = cloud(&["o1"]);
        pending["revoke_pending"] = json!(true);
        assert_eq!(after_cloud(&pending), Err(Outcome::Unavailable));
        assert_eq!(
            after_cloud(&json!({"state":"no_session"})),
            Err(Outcome::Unavailable)
        );
        assert_eq!(
            after_cloud(&json!({"state":"listed","organizations":[{"id":"","name":"x"}]})),
            Err(Outcome::Unavailable),
            "a malformed row is unreported"
        );
    }

    #[test]
    fn the_owned_listing_must_be_that_one_with_a_revision() {
        let owned = json!({"organizations":[{"id":"o1","name":"Org"}],"orgs_revision":"r1"});
        assert_eq!(after_owned(&owned, "o1"), Ok("r1".into()));
        assert_eq!(after_owned(&owned, "o2"), Err(Outcome::NeedsChoice));
        let empty_revision = json!({"organizations":[{"id":"o1","name":"Org"}],"orgs_revision":""});
        assert_eq!(
            after_owned(&empty_revision, "o1"),
            Err(Outcome::NeedsChoice)
        );
    }

    #[test]
    fn a_rail_in_any_other_state_needs_the_chooser() {
        assert_eq!(
            after_rails(&rail(json!({})), "o1"),
            Ok(Rail::Link { version: 0 })
        );
        // A historical link (version moved on) needs the chooser too.
        assert_eq!(
            after_rails(&rail(json!({"version":3})), "o1"),
            Err(Outcome::NeedsChoice)
        );
        assert_eq!(after_rails(&linked("o1"), "o1"), Ok(Rail::AlreadyLinked));
        assert_eq!(after_rails(&linked("o2"), "o1"), Err(Outcome::NeedsChoice));
        for extra in [
            json!({"state":"waiting"}),
            json!({"request_state":"pending"}),
            json!({"unlink_requested":true}),
            json!({"recipient_unavailable":true}),
            json!({"recipient_ref_check":"mismatch"}),
        ] {
            assert_eq!(
                after_rails(&rail(extra.clone()), "o1"),
                Err(Outcome::NeedsChoice),
                "{extra}"
            );
        }
        assert_eq!(
            after_rails(
                &json!({"rails":[{"rail_id":"other","mode":"live","linked":false,"version":0}]}),
                "o1"
            ),
            Err(Outcome::Unavailable)
        );
        assert_eq!(after_rails(&json!({}), "o1"), Err(Outcome::Unavailable));
    }

    #[test]
    fn the_select_and_the_link_are_read_back() {
        let mut selected = cloud(&["o1"]);
        selected["current_organization_id"] = json!("o1");
        assert_eq!(after_selected(&selected, "o1"), Ok(()));
        assert_eq!(
            after_selected(&cloud(&["o1"]), "o1"),
            Err(Outcome::Unavailable)
        );
        assert_eq!(confirmed(&linked("o1"), "o1"), Outcome::Ready);
        assert_eq!(confirmed(&rail(json!({})), "o1"), Outcome::Unavailable);
    }
}
