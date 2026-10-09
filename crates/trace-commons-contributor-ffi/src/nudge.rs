//! The re-engagement nudge copy, for shells.
use super::*;

/// Every nudge string, as one JSON object of key to text, including both
/// candidates of each pending wording choice and every singular form.
/// Opens no store. Shells use it for the fixed strings (Settings rows,
/// offers, action labels); every sentence with a count in it arrives already
/// composed on `status.nudge.text`, `status.nudge.mark_text`,
/// `reengage_due` and `digest_due`. The returned JSON string is owned and
/// must be freed with `tc_string_free`. Every string is DRAFT, NEEDS
/// APPROVAL.
#[unsafe(no_mangle)]
pub extern "C" fn tc_nudge_copy_json() -> *mut c_char {
    guarded_string_no_err(|| {
        let table: serde_json::Map<String, serde_json::Value> =
            trace_commons_contributor::nudge_copy::NUDGE_COPY
                .iter()
                .map(|(key, text)| ((*key).to_string(), serde_json::Value::from(*text)))
                .collect();
        let json = serde_json::to_string(&table).unwrap_or_else(|_| "{}".to_string());
        Ok(to_owned_cstring(&json))
    })
}

/// A Traces row's tags, worded by the core from the row's own
/// `list_pending` fields: `entry_json` is a borrowed UTF-8 JSON object
/// carrying the entry's `mission_fit` and `credit_estimate` as the daemon
/// sent them (the whole entry, or just those two keys). Returns an owned
/// JSON object with `mission_fit`, `estimate_band`, `estimate_tier` and
/// `estimate_explainer`, each present only when there is something true to
/// draw: the mission tag above zero, the estimate only while `drawn` is
/// true. A NULL, unreadable or mistyped input answers `{}`, which draws
/// nothing. Free with `tc_string_free`. DRAFT, NEEDS APPROVAL wording.
///
/// # Safety
/// `entry_json`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_nudge_entry_tags_json(entry_json: *const c_char) -> *mut c_char {
    guarded_string_no_err(|| {
        let entry: serde_json::Value = if entry_json.is_null() {
            serde_json::Value::Null
        } else {
            unsafe { CStr::from_ptr(entry_json) }
                .to_str()
                .ok()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or(serde_json::Value::Null)
        };
        let mission_fit = entry.get("mission_fit").and_then(serde_json::Value::as_u64);
        let estimate = entry.get("credit_estimate").and_then(|e| {
            Some(trace_commons_contributor::nudge_render::EntryEstimate {
                low: e.get("low")?.as_f64()?,
                high: e.get("high")?.as_f64()?,
                tier: e
                    .get("tier")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                drawn: e
                    .get("drawn")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        });
        let tags =
            trace_commons_contributor::nudge_render::entry_tags(mission_fit, estimate.as_ref());
        let json = serde_json::to_string(&tags).unwrap_or_else(|_| "{}".to_string());
        Ok(to_owned_cstring(&json))
    })
}

/// The digest switch's Settings help, composed by the core:
/// `settings_json` is a borrowed UTF-8 JSON object carrying
/// `digest_schedule` and `digest_interval_secs` as `get_settings` sent them
/// (the whole response, or just those keys). Returns an owned JSON object
/// with `digest_help`, present only when there is a line to draw: the
/// evening line, or the interval in whole hours, singular at one. A NULL,
/// unreadable or mistyped input answers `{}`, which draws nothing. Free with
/// `tc_string_free`. DRAFT, NEEDS APPROVAL wording.
///
/// # Safety
/// `settings_json`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_nudge_digest_help_json(settings_json: *const c_char) -> *mut c_char {
    guarded_string_no_err(|| {
        let settings: serde_json::Value = if settings_json.is_null() {
            serde_json::Value::Null
        } else {
            unsafe { CStr::from_ptr(settings_json) }
                .to_str()
                .ok()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or(serde_json::Value::Null)
        };
        let evening = settings
            .get("digest_schedule")
            .and_then(|s| s.get("mode"))
            .and_then(serde_json::Value::as_str)
            == Some("evening");
        let interval = settings
            .get("digest_interval_secs")
            .and_then(serde_json::Value::as_u64);
        let mut out = serde_json::Map::new();
        if let Some(help) = trace_commons_contributor::nudge_render::digest_help(evening, interval)
        {
            out.insert("digest_help".to_string(), serde_json::Value::from(help));
        }
        let json = serde_json::to_string(&out).unwrap_or_else(|_| "{}".to_string());
        Ok(to_owned_cstring(&json))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_help(settings: Option<&str>) -> serde_json::Value {
        let owned = settings.map(|s| CString::new(s).unwrap());
        let result = unsafe {
            tc_nudge_digest_help_json(owned.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()))
        };
        assert!(!result.is_null());
        let value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { tc_string_free(result) };
        value
    }

    #[test]
    fn the_digest_help_is_composed_from_the_settings() {
        assert_eq!(
            digest_help(Some(
                r#"{"digest_interval_secs":3600,"digest_schedule":{"mode":"interval"}}"#
            ))["digest_help"],
            "At most one notification an hour, and none when nothing is waiting."
        );
        assert_eq!(
            digest_help(Some(r#"{"digest_interval_secs":21600}"#))["digest_help"],
            "At most one notification every 6 hours, and none when nothing is waiting."
        );
        assert_eq!(
            digest_help(Some(
                r#"{"digest_interval_secs":3600,"digest_schedule":{"mode":"evening","hour":18}}"#
            ))["digest_help"],
            trace_commons_contributor::nudge_copy::SETTING_DIGEST_HELP_EVENING
        );
        for nothing in [
            None,
            Some("not json"),
            Some(r#"{"digest_interval_secs":5400}"#),
            Some(r#"{"digest_interval_secs":"3600"}"#),
            Some("{}"),
        ] {
            assert_eq!(digest_help(nothing), serde_json::json!({}), "{nothing:?}");
        }
    }

    #[test]
    fn the_copy_export_carries_the_whole_table() {
        let result = tc_nudge_copy_json();
        assert!(!result.is_null());
        let copy: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        let table = trace_commons_contributor::nudge_copy::NUDGE_COPY;
        assert_eq!(copy.as_object().unwrap().len(), table.len());
        for (key, text) in table {
            assert_eq!(copy[*key], *text, "{key}");
        }
        assert_eq!(copy["NUDGE_BACKLOG_TITLE"], "{n} traces to review");
        unsafe { tc_string_free(result) };
    }

    fn tags(entry: Option<&str>) -> serde_json::Value {
        let owned = entry.map(|e| CString::new(e).unwrap());
        let result = unsafe {
            tc_nudge_entry_tags_json(owned.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()))
        };
        assert!(!result.is_null());
        let value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { tc_string_free(result) };
        value
    }

    #[test]
    fn a_row_gets_its_tags_from_its_own_wire_fields() {
        let value = tags(Some(
            r#"{"entry_id":"e1","mission_fit":1,"credit_estimate":{"low":2.0,"high":4.5,"tier":"higher","calibration":"lef1.t2/cq3","basis":"published","drawn":true}}"#,
        ));
        assert_eq!(value["mission_fit"], "Fits a mission");
        assert_eq!(value["estimate_band"], "Estimate: about 2 to 4.5 credit");
        assert_eq!(value["estimate_tier"], "Higher estimate");
        assert_eq!(
            value["estimate_explainer"],
            trace_commons_contributor::nudge_copy::ESTIMATE_EXPLAINER
        );
    }

    #[test]
    fn a_row_that_says_nothing_drawable_gets_no_tags() {
        let empty = serde_json::json!({});
        // Not drawn, an older daemon's missing `drawn`, a zero fit, no fields.
        assert_eq!(
            tags(Some(
                r#"{"mission_fit":0,"credit_estimate":{"low":1.0,"high":3.0,"calibration":"c","basis":"built_in","drawn":false}}"#
            )),
            empty
        );
        assert_eq!(
            tags(Some(
                r#"{"credit_estimate":{"low":1.0,"high":3.0,"tier":"higher","calibration":"c","basis":"published"}}"#
            )),
            empty
        );
        assert_eq!(tags(Some("{}")), empty);
        // Unreadable input fails closed.
        assert_eq!(tags(None), empty);
        assert_eq!(tags(Some("not json")), empty);
        assert_eq!(tags(Some(r#"{"mission_fit":"two"}"#)), empty);
    }
}
