//! K11 on Linux: the raw send, both enclaves, and where the witness came
//! from, as rows this shell lays out.
//!
//! Every fact is the daemon's (`route_disclosure`, `certificate_detail`) and
//! every sentence is the contributor core's: this module calls
//! `consent_copy::route_disclosure_for_wire`, the same function the C ABI's
//! `tc_route_disclosure_copy` wraps, and only turns its answer into rows. It
//! holds no words of its own -- `tests/shell_wording.rs` checks that.
//!
//! Pure, so it is tested here without a display.

use trace_commons_contributor::consent_copy;

/// One line of a disclosure, as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A heading for the block that follows.
    Heading(String),
    /// A sentence.
    Text(String),
    /// A label, then a machine value shown verbatim (monospace).
    Value(String, String),
    /// One label, then each machine value under it -- the witness's pins,
    /// which macOS lists under a single "Pinned measurements".
    Values(String, Vec<String>),
}

/// One widget, as `ui::fill_disclosure_rows` draws it: what a contributor
/// actually sees, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drawn {
    /// A small-caps label.
    Eyebrow(String),
    /// A wrapped sentence.
    Body(String),
    /// A machine value, verbatim, monospace and selectable.
    Mono(String),
}

/// The widgets `rows` draw, in order. `ui::fill_disclosure_rows` makes one
/// GTK widget per item and decides nothing else, so this is what is drawn.
#[must_use]
pub fn drawn(rows: &[Row]) -> Vec<Drawn> {
    let mut out = Vec::new();
    for row in rows {
        match row {
            Row::Heading(text) => out.push(Drawn::Eyebrow(text.clone())),
            Row::Text(text) => out.push(Drawn::Body(text.clone())),
            Row::Value(label, value) => {
                out.push(Drawn::Eyebrow(label.clone()));
                out.push(Drawn::Mono(value.clone()));
            }
            Row::Values(label, values) => {
                out.push(Drawn::Eyebrow(label.clone()));
                out.extend(values.iter().cloned().map(Drawn::Mono));
            }
        }
    }
    out
}

/// The Settings section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    /// The section title: the core's, whether or not the route could be
    /// read.
    pub title: String,
    /// The card's rows.
    pub rows: Vec<Row>,
}

/// The Settings section: its title, and its rows.
///
/// `facts` is the daemon's `route_disclosure` result, or `None` when the
/// call failed. Anything unreadable -- a failed call, or a route or origin
/// newer than this build -- is said as such and never drawn as some other
/// route.
#[must_use]
pub fn panel(facts: Option<&serde_json::Value>) -> Panel {
    let Some(payload) = facts.and_then(consent_copy::route_disclosure_for_wire) else {
        // Still titled, from the core's unreadable copy, so the section is
        // named on every shell: macOS, Windows and Tauri draw it too.
        let unreadable = consent_copy::disclosure_unreadable_copy();
        return Panel {
            title: unreadable.title.to_string(),
            rows: vec![Row::Text(unreadable.panel.to_string())],
        };
    };
    let copy = &payload["copy"];
    let facts = &payload["facts"];
    let mut rows = vec![Row::Text(text(&copy["route"]))];
    push_optional(&mut rows, &copy["local_filter"]);
    if let (Some(witness), false) = (copy["witness"].as_object(), facts["witness"].is_null()) {
        let field = |key: &str| text(&witness[key]);
        rows.push(Row::Heading(field("heading")));
        rows.push(Row::Value(
            field("address_label"),
            text(&facts["witness"]["url"]),
        ));
        rows.push(Row::Value(
            field("signing_label"),
            text(&facts["witness"]["signing_address"]),
        ));
        let pins = facts["witness"]["pinned_measurements"]
            .as_array()
            .into_iter()
            .flatten()
            .map(text)
            .collect();
        rows.push(Row::Values(field("measurements_label"), pins));
        rows.push(Row::Text(field("check")));
        push_optional(&mut rows, &witness["classifier"]);
        rows.push(Row::Text(field("origin")));
    }
    push_optional(&mut rows, &copy["attested_bodies"]);
    push_optional(&mut rows, &copy["receipts"]);
    Panel {
        title: text(&copy["title"]),
        rows,
    }
}

/// One session's rows for the preview sheet: the size before redaction and
/// after, where each goes, and -- for a session whose witness reviewed it --
/// the measurement and signer it was checked against.
///
/// `raw` and `would_send` are the preview's own sizes, already formatted.
/// `certificate` is the daemon's `certificate_detail` result, or `None`; a
/// verification other than `verified_at_review` is not worded as that one.
#[must_use]
pub fn session(
    facts: Option<&serde_json::Value>,
    raw: &str,
    would_send: &str,
    certificate: Option<&serde_json::Value>,
) -> Vec<Row> {
    let Some(payload) = facts.and_then(consent_copy::route_disclosure_for_wire) else {
        return vec![Row::Text(
            consent_copy::DISCLOSURE_SESSION_UNREADABLE.to_string(),
        )];
    };
    let copy = &payload["copy"];
    let session = &copy["session"];
    let mut rows = vec![
        Row::Heading(text(&session["heading"])),
        Row::Value(text(&session["before_label"]), raw.to_string()),
        Row::Text(text(&session["before_line"])),
    ];
    push_optional(&mut rows, &copy["local_filter"]);
    rows.push(Row::Value(
        text(&session["after_label"]),
        would_send.to_string(),
    ));
    rows.push(Row::Text(text(&session["after_line"])));
    if payload["facts"]["route"] == "witness" {
        rows.push(Row::Text(text(&copy["route"])));
    }
    if let Some((measurement, signer)) = certificate.and_then(verified_at_review) {
        let labels = consent_copy::certificate_detail_copy();
        rows.push(Row::Heading(labels.heading.to_string()));
        rows.push(Row::Value(
            labels.measurement_label.to_string(),
            measurement.to_string(),
        ));
        rows.push(Row::Value(
            labels.signer_label.to_string(),
            signer.to_string(),
        ));
        rows.push(Row::Text(labels.verified_at_review.to_string()));
    }
    rows
}

/// The measurement and signer of a certificate verified at review, or
/// `None`. The sentence under them says the certificate was checked against
/// "this measurement" and "this signing key", so a detail missing either --
/// null, absent or empty -- is not worded at all; macOS, Tauri and Windows
/// refuse the same detail.
fn verified_at_review(detail: &serde_json::Value) -> Option<(&str, &str)> {
    if detail["verification"] != "verified_at_review" {
        return None;
    }
    let present = |key: &str| detail[key].as_str().filter(|s| !s.is_empty());
    Some((present("witness_measurement")?, present("signer")?))
}

fn text(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn push_optional(rows: &mut Vec<Row>, value: &serde_json::Value) {
    if let Some(line) = value.as_str().filter(|s| !s.is_empty()) {
        rows.push(Row::Text(line.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn witness_facts() -> serde_json::Value {
        serde_json::json!({
            "route": "witness",
            "witness": {
                "state": "pinned", "url": "https://witness.example", "signing_address": "0xab",
                "pinned_measurements": ["mrtd=aa", "mrtd=bb"], "origin": "connected_inference",
            },
            "local_filter": null,
            "receipts": {"endpoint_configured": true, "check_attestation": true},
            "attested_bodies": true,
        })
    }

    /// What the Settings section draws for the witness route, widget by
    /// widget: the title over the card, and every pin under one
    /// "Pinned measurements" label, as macOS lays it out.
    #[test]
    fn the_settings_section_draws_every_pin_under_one_label() {
        use consent_copy as c;
        let panel = panel(Some(&witness_facts()));
        assert_eq!(panel.title, c::DISCLOSURE_TITLE);
        let eyebrow = |s: &str| Drawn::Eyebrow(s.into());
        let body = |s: &str| Drawn::Body(s.into());
        let mono = |s: &str| Drawn::Mono(s.into());
        assert_eq!(
            drawn(&panel.rows),
            vec![
                body(c::AUTO_RAW_SEND_BOTH_ENCLAVES),
                eyebrow(c::DISCLOSURE_WITNESS_HEADING),
                eyebrow(c::DISCLOSURE_WITNESS_ADDRESS_LABEL),
                mono("https://witness.example"),
                eyebrow(c::DISCLOSURE_WITNESS_SIGNING_LABEL),
                mono("0xab"),
                eyebrow(c::DISCLOSURE_WITNESS_MEASUREMENTS_LABEL),
                mono("mrtd=aa"),
                mono("mrtd=bb"),
                body(c::DISCLOSURE_WITNESS_CHECK),
                body(c::DISCLOSURE_WITNESS_CLASSIFIER),
                body(c::DISCLOSURE_ORIGIN_CONNECTED_INFERENCE),
                body(c::DISCLOSURE_ATTESTED_BODIES),
                body(c::DISCLOSURE_RECEIPTS_CHECKED),
            ]
        );
    }

    /// An unreadable disclosure is still named: the core's title heads its
    /// one line, as on macOS, Windows and Tauri, so the section is not an
    /// anonymous warning.
    #[test]
    fn an_unreadable_disclosure_is_titled_from_the_core() {
        let panel = panel(None);
        assert_eq!(
            panel.title,
            consent_copy::disclosure_unreadable_copy().title
        );
        assert_eq!(
            drawn(&panel.rows),
            vec![Drawn::Body(consent_copy::DISCLOSURE_UNREADABLE.into())]
        );
    }

    /// What the preview sheet draws for one witness-reviewed session.
    #[test]
    fn a_session_draws_its_sizes_route_and_certificate() {
        use consent_copy as c;
        let detail = serde_json::json!({
            "state": "held", "verification": "verified_at_review",
            "witness_measurement": "mrtd=aa", "signer": "0xab",
        });
        let labels = c::certificate_detail_copy();
        let rows = session(Some(&witness_facts()), "1 KB", "4 KB", Some(&detail));
        let eyebrow = |s: &str| Drawn::Eyebrow(s.into());
        let body = |s: &str| Drawn::Body(s.into());
        let mono = |s: &str| Drawn::Mono(s.into());
        assert_eq!(
            drawn(&rows),
            vec![
                eyebrow(c::DISCLOSURE_SESSION_HEADING),
                eyebrow(c::DISCLOSURE_SESSION_BEFORE_LABEL),
                mono("1 KB"),
                body(c::DISCLOSURE_SESSION_BEFORE_WITNESS),
                eyebrow(c::DISCLOSURE_SESSION_AFTER_LABEL),
                mono("4 KB"),
                body(c::DISCLOSURE_SESSION_AFTER),
                body(c::AUTO_RAW_SEND_BOTH_ENCLAVES),
                eyebrow(labels.heading),
                eyebrow(labels.measurement_label),
                mono("mrtd=aa"),
                eyebrow(labels.signer_label),
                mono("0xab"),
                body(labels.verified_at_review),
            ]
        );
    }

    fn texts(rows: &[Row]) -> Vec<&str> {
        rows.iter()
            .filter_map(|r| match r {
                Row::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_witness_route_shows_the_witness_its_pins_and_origin_in_the_cores_words() {
        let Panel { title, rows } = panel(Some(&witness_facts()));
        assert_eq!(title, consent_copy::DISCLOSURE_TITLE);
        let lines = texts(&rows);
        assert_eq!(lines[0], consent_copy::AUTO_RAW_SEND_BOTH_ENCLAVES);
        assert!(lines.contains(&consent_copy::DISCLOSURE_WITNESS_CHECK));
        assert!(lines.contains(&consent_copy::DISCLOSURE_WITNESS_CLASSIFIER));
        assert!(lines.contains(&consent_copy::DISCLOSURE_ORIGIN_CONNECTED_INFERENCE));
        assert!(lines.contains(&consent_copy::DISCLOSURE_ATTESTED_BODIES));
        assert!(lines.contains(&consent_copy::DISCLOSURE_RECEIPTS_CHECKED));
        assert!(
            rows.contains(&Row::Values(
                consent_copy::DISCLOSURE_WITNESS_MEASUREMENTS_LABEL.into(),
                vec!["mrtd=aa".into(), "mrtd=bb".into()]
            )),
            "every pin, verbatim, under one label"
        );
        assert!(rows.contains(&Row::Value(
            consent_copy::DISCLOSURE_WITNESS_ADDRESS_LABEL.into(),
            "https://witness.example".into()
        )));
    }

    #[test]
    fn the_local_route_shows_its_filter_and_no_witness() {
        let facts = serde_json::json!({
            "route": "local", "witness": null, "local_filter": "near_ai",
            "receipts": {"endpoint_configured": false, "check_attestation": false},
            "attested_bodies": false,
        });
        let rows = panel(Some(&facts)).rows;
        assert_eq!(
            texts(&rows),
            vec![
                consent_copy::DISCLOSURE_ROUTE_LOCAL,
                consent_copy::DISCLOSURE_LOCAL_FILTER_NEAR_AI
            ]
        );
    }

    /// A failed call, or a route or origin newer than this build, is said as
    /// unreadable -- never rendered as the nearest route.
    #[test]
    fn anything_unreadable_says_so() {
        let mut newer = witness_facts();
        newer["witness"]["origin"] = serde_json::json!("an_operator");
        for facts in [None, Some(&newer)] {
            let rows = panel(facts).rows;
            assert_eq!(
                rows,
                vec![Row::Text(consent_copy::DISCLOSURE_UNREADABLE.into())]
            );
            assert_eq!(
                session(facts, "1 KB", "4 KB", None),
                vec![Row::Text(
                    consent_copy::DISCLOSURE_SESSION_UNREADABLE.into()
                )]
            );
        }
    }

    #[test]
    fn a_session_shows_both_sizes_the_route_and_its_certificate() {
        let detail = serde_json::json!({
            "state": "held", "verification": "verified_at_review",
            "witness_measurement": "mrtd=aa", "signer": "0xab",
        });
        let rows = session(Some(&witness_facts()), "1 KB", "4 KB", Some(&detail));
        assert!(rows.contains(&Row::Value(
            consent_copy::DISCLOSURE_SESSION_BEFORE_LABEL.into(),
            "1 KB".into()
        )));
        assert!(rows.contains(&Row::Value(
            consent_copy::DISCLOSURE_SESSION_AFTER_LABEL.into(),
            "4 KB".into()
        )));
        let lines = texts(&rows);
        assert!(lines.contains(&consent_copy::DISCLOSURE_SESSION_BEFORE_WITNESS));
        assert!(lines.contains(&consent_copy::AUTO_RAW_SEND_BOTH_ENCLAVES));
        let labels = consent_copy::certificate_detail_copy();
        assert!(lines.contains(&labels.verified_at_review));
        assert!(rows.contains(&Row::Value(labels.signer_label.into(), "0xab".into())));
        // A verification this build has no sentence for is not worded as
        // the one it has.
        let mut other = detail;
        other["verification"] = serde_json::json!("verified_now");
        let rows = session(Some(&witness_facts()), "1 KB", "4 KB", Some(&other));
        assert!(!texts(&rows).contains(&labels.verified_at_review));
    }

    /// "Checked against this signing key" is never drawn beside an empty
    /// key, nor "this measurement" beside an empty measurement: a detail
    /// missing either one is dropped whole, as macOS
    /// (`CertificateDetail.decode`), Tauri and Windows drop it. The rest of
    /// the session's rows still stand.
    #[test]
    fn a_certificate_missing_its_measurement_or_signer_is_not_worded_as_checked() {
        let labels = consent_copy::certificate_detail_copy();
        let without_one = [
            ("signer", serde_json::Value::Null),
            ("signer", serde_json::json!("")),
            ("witness_measurement", serde_json::Value::Null),
            ("witness_measurement", serde_json::json!("")),
        ];
        for (field, value) in without_one {
            let mut detail = serde_json::json!({
                "state": "held", "verification": "verified_at_review",
                "witness_measurement": "mrtd=aa", "signer": "0xab",
            });
            detail[field] = value.clone();
            let rows = session(Some(&witness_facts()), "1 KB", "4 KB", Some(&detail));
            assert_eq!(
                rows,
                session(Some(&witness_facts()), "1 KB", "4 KB", None),
                "{field} = {value}: no certificate rows at all"
            );
            assert!(!texts(&rows).contains(&labels.verified_at_review));
        }
    }
}
