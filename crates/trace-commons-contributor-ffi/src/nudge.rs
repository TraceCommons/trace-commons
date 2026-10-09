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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            copy["NUDGE_BACKLOG_TITLE"],
            "{n} unpurposed traces are waiting"
        );
        unsafe { tc_string_free(result) };
    }
}
