// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for `trace-commons-staging-pins`. Every key is generated here and
//! every measurement is synthetic.

use clap::CommandFactory as _;
use rand::RngCore as _;
use trace_commons_attestation::staging_pins::{self, PinRefusal};

use super::*;

/// The refuse-everything placeholder allowlist: the shape the repository
/// shipped before the reviewed values (#1578), kept here for the tests that
/// exercise it.
const PLACEHOLDER_ALLOWLIST: &str = r#"{
  "rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []},
  "compose_actions": {"compose_manager_images": [], "compose_files": []}
}"#;

/// 2026-10-21T18:00:00Z.
const NOW: u64 = 1_792_605_600;
const DAY: u64 = 86_400;

fn seed_hex() -> String {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    hex::encode(seed)
}

fn public_key(seed_hex: &str) -> Vec<u8> {
    let pair = Ed25519KeyPair::from_seed_unchecked(&hex::decode(seed_hex).unwrap()).unwrap();
    pair.public_key().as_ref().to_vec()
}

/// A synthetic set in the S16 form, canonical order.
fn synthetic_set(byte: &str) -> String {
    ["mrtd", "mrconfigid", "rtmr0", "rtmr1", "rtmr2"]
        .iter()
        .map(|field| format!("{field}={}", byte.repeat(48)))
        .collect::<Vec<_>>()
        .join(",")
}

/// The staging ingest's own pin, production's form.
fn synthetic_ingest_pin() -> String {
    ["mrtd", "rtmr0", "rtmr1", "rtmr2", "rtmr3"]
        .iter()
        .map(|field| format!("{field}={}", "c3".repeat(48)))
        .collect::<Vec<_>>()
        .join(",")
}

fn sets_text() -> String {
    format!(
        "{}\n{}\ningest={}\n",
        synthetic_set("a1"),
        synthetic_set("b2"),
        synthetic_ingest_pin()
    )
}

const ALLOWLIST: &str = r#"{
  "rtmr3": {
    "app_id": "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    "mr_kms": "3b3b",
    "os_image_hash": "0505",
    "key_provider": "6b6d73",
    "markers": [
      {"name": "system-preparing", "payload": ""},
      {"name": "boot-mr-done", "payload": ""},
      {"name": "storage-fs", "payload": "00"},
      {"name": "system-ready", "payload": "00"}
    ]
  },
  "compose_actions": {
    "compose_manager_images": [
      "registry.example/compose-manager@sha256:abababababababababababababababababababababababababababababababab"
    ],
    "compose_files": [
      {
        "file": "example/models.yaml",
        "file_sha256": "f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1",
        "commit": "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1"
      }
    ]
  }
}"#;

fn request<'a>(sets_text: &'a str, sequence: u64, served: Option<&'a [u8]>) -> SignRequest<'a> {
    SignRequest {
        sets_text,
        allowlist_text: ALLOWLIST,
        sequence,
        ttl_days: 7,
        served,
        now: NOW,
    }
}

#[test]
fn sign_then_verify_round_trips() {
    let seed = seed_hex();
    let sets = sets_text();
    let signed = sign(format!("{seed}\n").as_bytes(), &request(&sets, 42, None))
        .expect("a well-formed request signs");

    let verified = staging_pins::verify(
        &signed.document,
        &public_key(&seed),
        NOW,
        &staging_gateway_host(),
    )
    .expect("the app's verifier accepts it");
    assert_eq!(verified.sequence, 42);
    assert_eq!(verified.issued_at, NOW);
    assert_eq!(verified.expires_at, NOW + 7 * DAY);
    assert_eq!(
        verified
            .sets
            .iter()
            .map(|s| s.to_pin_string())
            .collect::<Vec<_>>(),
        [synthetic_set("a1"), synthetic_set("b2")]
    );
    assert_eq!(
        signed.expected_measurements_line,
        format!(
            "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS={}",
            synthetic_ingest_pin()
        ),
        "the ingest's pin is the production-form line, never an S16 set"
    );

    // And through this tool's own verify, which is the app's check.
    assert!(verify_document(&signed.document, &hex::encode(public_key(&seed)), NOW).is_ok());
    // Another key refuses it.
    assert_eq!(
        verify_document(&signed.document, &hex::encode(public_key(&seed_hex())), NOW).unwrap_err(),
        PinRefusal::UnknownKey.label()
    );
}

#[test]
fn the_payload_names_the_staging_gateway_host() {
    // The bare host, as the app compares it, of the compiled-in staging
    // NEAR AI API base -- not a URL, not a path.
    assert_eq!(staging_gateway_host(), "cloud-stg-api.near.ai");
    let host = reqwest::Url::parse(trace_commons_server::near_ai_api::NEAR_AI_API_BASE_URL)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    assert_eq!(staging_gateway_host(), host);
}

#[test]
fn sign_refuses_a_sequence_not_above_the_served_one() {
    let seed = seed_hex();
    let sets = sets_text();
    let served = sign(seed.as_bytes(), &request(&sets, 10, None))
        .unwrap()
        .document;

    for sequence in [0, 9, 10] {
        assert_eq!(
            sign(seed.as_bytes(), &request(&sets, sequence, Some(&served)))
                .err()
                .as_deref(),
            Some("sequence_not_above_served"),
            "sequence {sequence} against a served 10"
        );
    }
    assert!(sign(seed.as_bytes(), &request(&sets, 11, Some(&served))).is_ok());

    // An expired served document still names its sequence: the guard does
    // not lapse with the document it guards against rolling back.
    let later = SignRequest {
        now: NOW + 30 * DAY,
        ..request(&sets, 10, Some(&served))
    };
    assert_eq!(
        sign(seed.as_bytes(), &later).err().as_deref(),
        Some("sequence_not_above_served")
    );
}

#[test]
fn a_served_document_without_a_readable_sequence_is_refused() {
    let seed = seed_hex();
    let sets = sets_text();
    for served in [
        &b"not json"[..],
        br#"{"payload":"!!","signature":"","key_id":""}"#,
        br#"{"signature":"","key_id":""}"#,
    ] {
        assert_eq!(
            sign(seed.as_bytes(), &request(&sets, 99, Some(served)))
                .err()
                .as_deref(),
            Some("served_document_unreadable")
        );
    }
}

#[test]
fn sign_refuses_a_ttl_over_fourteen_days() {
    let seed = seed_hex();
    let sets = sets_text();
    for (ttl_days, ok) in [
        (0, false),
        (1, true),
        (14, true),
        (15, false),
        (u64::MAX, false),
    ] {
        let req = SignRequest {
            ttl_days,
            ..request(&sets, 1, None)
        };
        let result = sign(seed.as_bytes(), &req);
        if ok {
            assert!(result.is_ok(), "ttl {ttl_days} days signs");
        } else {
            assert_eq!(
                result.err().as_deref(),
                Some("ttl_out_of_range"),
                "ttl {ttl_days} days"
            );
        }
    }
}

#[test]
fn sign_refuses_sets_that_do_not_parse_or_are_none() {
    let seed = seed_hex();
    assert_eq!(
        sign(seed.as_bytes(), &request("\n  \n", 1, None))
            .err()
            .as_deref(),
        Some("no_sets")
    );
    let bad = format!("{}\nmrtd=zz\n", synthetic_set("a1"));
    assert_eq!(
        sign(seed.as_bytes(), &request(&bad, 1, None))
            .err()
            .as_deref(),
        Some("set_unparseable:line_2")
    );
    // A repeated set is signed once.
    let twice = format!(
        "{}\n{}\ningest={}\n",
        synthetic_set("a1"),
        synthetic_set("a1"),
        synthetic_ingest_pin()
    );
    let signed = sign(seed.as_bytes(), &request(&twice, 1, None)).unwrap();
    let verified = staging_pins::verify(
        &signed.document,
        &public_key(&seed),
        NOW,
        &staging_gateway_host(),
    )
    .unwrap();
    assert_eq!(verified.sets.len(), 1);
}

#[test]
fn sign_refuses_a_missing_or_malformed_key() {
    let sets = sets_text();
    for (stdin, label) in [
        (String::new(), "signing_key_missing"),
        ("\n".to_string(), "signing_key_missing"),
        ("ab".repeat(31), "signing_key_malformed"),
        (format!("{}zz", "ab".repeat(31)), "signing_key_malformed"),
        ("ab".repeat(33), "signing_key_malformed"),
    ] {
        assert_eq!(
            sign(stdin.as_bytes(), &request(&sets, 1, None))
                .err()
                .as_deref(),
            Some(label)
        );
    }
}

#[test]
fn sign_never_reads_a_key_from_argv() {
    // No argument of any subcommand that signs or derives takes a key: the
    // key arrives on stdin and nowhere else. `verify` takes the *public*
    // key, which is not secret.
    let command = Cli::command();
    for sub in command.get_subcommands() {
        for arg in sub.get_arguments() {
            let id = arg.get_id().as_str().to_ascii_lowercase();
            let names = [
                Some(id.clone()),
                arg.get_long().map(str::to_ascii_lowercase),
            ];
            for name in names.into_iter().flatten() {
                let secret_like = ["key", "seed", "secret", "private", "password", "token"]
                    .iter()
                    .any(|word| name.contains(word));
                assert!(
                    !secret_like || (sub.get_name() == "verify" && name.contains("public")),
                    "{} --{name} would put a secret on argv",
                    sub.get_name()
                );
            }
        }
    }
    let seed = seed_hex();
    for flag in ["--key", "--seed", "--private-key", "--signing-key"] {
        assert!(
            Cli::try_parse_from([
                "trace-commons-staging-pins",
                "sign",
                "--sets",
                "s",
                "--sequence",
                "1",
                "--ttl-days",
                "7",
                "--first",
                flag,
                &seed,
            ])
            .is_err(),
            "{flag} is refused"
        );
    }
    // And `public-key` takes nothing at all.
    assert!(Cli::try_parse_from(["trace-commons-staging-pins", "public-key", &seed]).is_err());
}

#[test]
fn sign_requires_saying_whether_a_set_is_served() {
    // Neither `--current` nor `--first`: refused, so a forgotten served file
    // is never read as "nothing served yet".
    assert!(
        Cli::try_parse_from([
            "trace-commons-staging-pins",
            "sign",
            "--sets",
            "s",
            "--sequence",
            "1",
            "--ttl-days",
            "7",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "trace-commons-staging-pins",
            "sign",
            "--sets",
            "s",
            "--sequence",
            "1",
            "--ttl-days",
            "7",
            "--first",
            "--current",
            "c.json",
        ])
        .is_err()
    );
}

#[test]
fn nothing_logs_a_key_or_a_measurement() {
    let seed = seed_hex();
    let key = hex::encode(public_key(&seed));
    let sets = sets_text();
    let signed = sign(seed.as_bytes(), &request(&sets, 5, None)).unwrap();
    let served = signed.document.clone();

    let mut logged = vec![
        signed.summary.clone(),
        verify_document(&signed.document, &key, NOW).unwrap(),
    ];
    for refusal in [
        sign(seed.as_bytes(), &request(&sets, 5, Some(&served))).unwrap_err(),
        sign(b"", &request(&sets, 6, None)).unwrap_err(),
        sign(format!("{seed}0").as_bytes(), &request(&sets, 6, None)).unwrap_err(),
        sign(seed.as_bytes(), &request("mrtd=zz", 6, None)).unwrap_err(),
        sign(
            seed.as_bytes(),
            &SignRequest {
                allowlist_text: "{}",
                ..request(&sets, 6, None)
            },
        )
        .unwrap_err(),
        verify_document(&signed.document, &key, NOW + 30 * DAY).unwrap_err(),
        verify_document(b"{}", &key, NOW).unwrap_err(),
        verify_document(&signed.document, "not-hex", NOW).unwrap_err(),
    ] {
        logged.push(refusal);
    }

    for line in &logged {
        assert!(!line.contains(&seed), "a log line carries the seed: {line}");
        assert!(
            !line.contains(&key),
            "a log line carries the public key: {line}"
        );
        assert!(!line.contains(&"a1".repeat(48)), "a measurement: {line}");
        assert!(!line.contains(&"b2".repeat(48)), "a measurement: {line}");
        assert!(!line.contains(&"c3".repeat(48)), "a measurement: {line}");
        assert!(!line.contains("3b3b"), "an allowlist value: {line}");
        assert!(!line.contains("registry.example"), "an image: {line}");
        assert!(
            !line.contains("example/models.yaml"),
            "a compose file: {line}"
        );
        assert!(!line.contains(&"f1".repeat(32)), "a file hash: {line}");
        assert!(!line.contains(&"c1".repeat(20)), "a commit: {line}");
    }
    // The summary identifies the document by hash, and the key by its id.
    assert!(signed.summary.contains("payload_sha256=sha256:"));
    assert!(signed.summary.contains(&format!(
        "key_id={}",
        staging_pins::key_id(&public_key(&seed))
    )));
}

#[test]
fn registers_parse_by_name_and_refuse_unknowns_and_repeats() {
    use trace_commons_attestation::measurements::MeasurementField;
    assert_eq!(
        parse_registers("mrtd, RTMR0,mrconfigid").unwrap(),
        [
            MeasurementField::Mrtd,
            MeasurementField::Rtmr0,
            MeasurementField::MrConfigId
        ]
    );
    assert_eq!(
        parse_registers("mrtd,rtmr0,rtmr1,rtmr2,mrconfigid").unwrap(),
        trace_commons_server::near_attestation::pin_observe::DEFAULT_REGISTERS
    );
    assert_eq!(
        parse_registers("mrtd,rtmr9").unwrap_err(),
        "registers_unknown"
    );
    assert_eq!(
        parse_registers("mrtd,mrtd").unwrap_err(),
        "registers_repeated"
    );
    assert_eq!(parse_registers("").unwrap_err(), "registers_unknown");
}

#[test]
fn observe_defaults_to_the_shared_register_list() {
    // The default is DEFAULT_REGISTERS itself, not a second copy of it. A
    // review file is required: observe always writes what it saw.
    assert!(
        Cli::try_parse_from(["trace-commons-staging-pins", "observe", "--model", "m"]).is_err()
    );
    let cli = Cli::try_parse_from([
        "trace-commons-staging-pins",
        "observe",
        "--model",
        "m",
        "--review",
        "review.json",
    ])
    .expect("observe with only a model parses");
    let Command::Observe { registers, .. } = cli.command else {
        panic!("parsed as observe");
    };
    assert_eq!(
        observe_registers(registers.as_deref()).unwrap(),
        trace_commons_server::near_attestation::pin_observe::DEFAULT_REGISTERS
    );
    assert_eq!(
        observe_registers(Some("mrtd")).unwrap(),
        [trace_commons_attestation::measurements::MeasurementField::Mrtd]
    );
}

#[test]
fn an_existing_output_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("current.json");
    std::fs::write(&out, b"served bytes").unwrap();

    assert_eq!(
        write_new(&out, b"new document").unwrap_err(),
        "output_exists"
    );
    assert_eq!(std::fs::read(&out).unwrap(), b"served bytes");

    let fresh = dir.path().join("next.json");
    write_new(&fresh, b"new document").expect("a new path is written");
    assert_eq!(std::fs::read(&fresh).unwrap(), b"new document");
}

#[test]
fn the_default_registers_are_the_s16_set() {
    use trace_commons_attestation::measurements::MeasurementField;
    // Owner decision S16: mrtd, rtmr0-2 and mrconfigid, with RTMR3 replayed
    // rather than pinned. It ships only with the replay, in this change.
    assert_eq!(
        trace_commons_server::near_attestation::pin_observe::DEFAULT_REGISTERS,
        [
            MeasurementField::Mrtd,
            MeasurementField::Rtmr0,
            MeasurementField::Rtmr1,
            MeasurementField::Rtmr2,
            MeasurementField::MrConfigId,
        ]
    );
    assert!(
        !trace_commons_server::near_attestation::pin_observe::DEFAULT_REGISTERS
            .contains(&MeasurementField::Rtmr3)
    );
    // And it is exactly what a v2 set must name at least.
    assert_eq!(
        trace_commons_server::near_attestation::pin_observe::DEFAULT_REGISTERS,
        staging_pins::REQUIRED_REGISTERS
    );
    // The staging ingest's pin keeps production's registers (spec 5.9).
    assert_eq!(
        trace_commons_server::near_attestation::pin_observe::INGEST_REGISTERS,
        [
            MeasurementField::Mrtd,
            MeasurementField::Rtmr0,
            MeasurementField::Rtmr1,
            MeasurementField::Rtmr2,
            MeasurementField::Rtmr3,
        ]
    );
}

#[test]
fn sign_embeds_the_allowlist_file() {
    let seed = seed_hex();
    let sets = sets_text();
    let signed = sign(seed.as_bytes(), &request(&sets, 9, None)).expect("signs");
    let verified = staging_pins::verify(
        &signed.document,
        &public_key(&seed),
        NOW,
        &staging_gateway_host(),
    )
    .unwrap();
    let file: serde_json::Value = serde_json::from_str(ALLOWLIST).unwrap();
    assert_eq!(verified.allowlist.to_json(), file);
    assert!(signed.summary.contains("allowlist_sha256=sha256:"));

    // The placeholder the repository ships signs: it is the one allowlist
    // with values missing that `sign` accepts, and it refuses every entry.
    let req = SignRequest {
        allowlist_text: PLACEHOLDER_ALLOWLIST,
        ..request(&sets, 9, None)
    };
    assert!(sign(seed.as_bytes(), &req).is_ok());

    // An allowlist filled in only in part signs nothing: an empty value or
    // an unlisted marker would refuse every entry the operator meant to
    // allow, and nothing else in the signed document would say why.
    let full: serde_json::Value = serde_json::from_str(ALLOWLIST).unwrap();
    let mut partial: Vec<serde_json::Value> = Vec::new();
    for field in ["app_id", "mr_kms", "os_image_hash", "key_provider"] {
        let mut case = full.clone();
        case["rtmr3"][field] = serde_json::json!("");
        partial.push(case);
    }
    let mut three_markers = full.clone();
    three_markers["rtmr3"]["markers"]
        .as_array_mut()
        .unwrap()
        .pop();
    partial.push(three_markers);
    let mut placeholder_with_actions: serde_json::Value =
        serde_json::from_str(PLACEHOLDER_ALLOWLIST).unwrap();
    placeholder_with_actions["compose_actions"] = full["compose_actions"].clone();
    partial.push(placeholder_with_actions);
    // RTMR3 filled in, a compose list left empty: every live record has a
    // compose_manager_started and a compose_up, so this refuses them all.
    for list in ["compose_manager_images", "compose_files"] {
        let mut case = full.clone();
        case["compose_actions"][list] = serde_json::json!([]);
        partial.push(case);
    }
    let mut placeholder_with_one_image: serde_json::Value =
        serde_json::from_str(PLACEHOLDER_ALLOWLIST).unwrap();
    placeholder_with_one_image["compose_actions"]["compose_manager_images"] =
        full["compose_actions"]["compose_manager_images"].clone();
    partial.push(placeholder_with_one_image);
    for case in partial {
        let text = case.to_string();
        let req = SignRequest {
            allowlist_text: &text,
            ..request(&sets, 9, None)
        };
        assert_eq!(
            sign(seed.as_bytes(), &req).err().as_deref(),
            Some("allowlist_incomplete"),
            "{case}"
        );
    }

    // A file that is not an allowlist signs nothing.
    for (text, label) in [
        ("not json", "allowlist_malformed"),
        ("{}", "allowlist_malformed"),
        (
            r#"{"rtmr3": {"app_id": "AB", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []}, "compose_actions": {"compose_manager_images": [], "compose_files": []}}"#,
            "allowlist_malformed",
        ),
        // The #1575 list form of compose_actions is not the v2 schema.
        (
            r#"{"rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []}, "compose_actions": []}"#,
            "allowlist_malformed",
        ),
    ] {
        let req = SignRequest {
            allowlist_text: text,
            ..request(&sets, 9, None)
        };
        assert_eq!(sign(seed.as_bytes(), &req).err().as_deref(), Some(label));
    }
}

#[test]
fn sign_refuses_sets_without_the_s16_registers_or_an_ingest_pin() {
    let seed = seed_hex();
    // A production-form set is not a v2 set: it does not pin mrconfigid.
    let production = format!(
        "{}\ningest={}\n",
        synthetic_ingest_pin(),
        synthetic_ingest_pin()
    );
    assert_eq!(
        sign(seed.as_bytes(), &request(&production, 1, None))
            .err()
            .as_deref(),
        Some("set_missing_register:line_1")
    );
    // No ingest line: the ingest cannot be pinned to an S16 set.
    let no_ingest = format!("{}\n", synthetic_set("a1"));
    assert_eq!(
        sign(seed.as_bytes(), &request(&no_ingest, 1, None))
            .err()
            .as_deref(),
        Some("ingest_pin_missing")
    );
    // An ingest line in the S16 form, or two ingest lines.
    for text in [
        format!("{}\ningest={}\n", synthetic_set("a1"), synthetic_set("a1")),
        format!(
            "{}\ningest={}\ningest={}\n",
            synthetic_set("a1"),
            synthetic_ingest_pin(),
            synthetic_ingest_pin()
        ),
        format!("{}\ningest=mrtd=zz\n", synthetic_set("a1")),
    ] {
        assert_eq!(
            sign(seed.as_bytes(), &request(&text, 1, None))
                .err()
                .as_deref(),
            Some("ingest_pin_malformed"),
            "{text}"
        );
    }
}
