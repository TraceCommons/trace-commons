//! Response-driven enrollment completion, independent of GTK widget access.

use std::cell::Cell;

pub(super) fn complete(
    result: Result<serde_json::Value, String>,
    busy: &Cell<bool>,
    report: impl FnOnce(&str, bool),
    show_consent: impl FnOnce(),
) {
    busy.set(false);
    match result {
        Ok(value) if value.get("enrolled").and_then(serde_json::Value::as_bool) == Some(true) => {
            report("", false);
            show_consent();
        }
        Ok(_) => report(
            trace_commons_contributor::private_inference_copy::NEAR_AI_ENROLL_UNAVAILABLE,
            true,
        ),
        Err(label) => report(&label, true),
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::onboarding_nearai::enrollment_result::complete;
    use std::cell::{Cell, RefCell};

    #[test]
    fn enrollment_advances_only_on_explicit_success() {
        for (result, expected) in [
            (Ok(serde_json::json!({"enrolled": true})), true),
            (Ok(serde_json::json!({"enrolled": false})), false),
            (Ok(serde_json::json!({})), false),
            (Ok(serde_json::json!({"enrolled": "true"})), false),
            (Ok(serde_json::json!({"enrolled": null})), false),
            (Ok(serde_json::json!([])), false),
            (Err("daemon-unavailable".into()), false),
        ] {
            let busy = Cell::new(true);
            let calls = RefCell::new(Vec::new());
            let reported = RefCell::new(None);
            complete(
                result,
                &busy,
                |label, refused| {
                    assert!(!busy.get());
                    calls.borrow_mut().push("report");
                    *reported.borrow_mut() = Some((label.to_owned(), refused));
                },
                || {
                    assert!(!busy.get());
                    calls.borrow_mut().push("show-consent");
                },
            );
            assert!(!busy.get());
            assert_eq!(reported.borrow().as_ref().unwrap().1, !expected);
            assert_eq!(
                *calls.borrow(),
                if expected {
                    vec!["report", "show-consent"]
                } else {
                    vec!["report"]
                }
            );
        }
    }

    #[test]
    fn a_daemon_refusal_is_preserved_and_malformed_success_is_unavailable() {
        let busy = Cell::new(true);
        for (result, expected) in [
            (Err("synthetic-refusal".into()), "synthetic-refusal"),
            (
                Ok(serde_json::Value::Null),
                trace_commons_contributor::private_inference_copy::NEAR_AI_ENROLL_UNAVAILABLE,
            ),
        ] {
            complete(
                result,
                &busy,
                |label, refused| {
                    assert_eq!(label, expected);
                    assert!(refused);
                },
                || panic!("a refusal must not reach consent"),
            );
        }
    }
}
