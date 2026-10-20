//! Corpus: real-looking source must pass through untouched.
//!
//! `docs/PRIVACY.md` §6. The failure mode this guards against is not a privacy
//! failure at all — it is the filter substituting values that are *load-bearing
//! code*, so the model writes something that does not compile and the user
//! blames the model.
//!
//! Every fixture below is the kind of thing that actually appears in a repo a
//! coding agent is working on.

use ironwire_privacy::mint::Class;
use ironwire_privacy::{Detector, Salt, Tiers};

fn deterministic_tiers() -> Tiers {
    Tiers {
        secrets: true,
        named_values: Vec::new(),
        pii: false,
    }
}

/// Files a coding agent routinely reads, none of which contain a real secret.
const CLEAN_FIXTURES: &[(&str, &str)] = &[
    (
        "a test asserting on a documentation email",
        r#"
#[test]
fn parses_an_address() {
    let user = User::parse("user@example.com").unwrap();
    assert_eq!(user.domain(), "example.com");
    assert_eq!(User::parse("admin@example.org").unwrap().local(), "admin");
}
"#,
    ),
    (
        "a config full of private and documentation addresses",
        r#"
[network]
bind = "127.0.0.1:8080"
gateway = "192.168.1.1"
peers = ["10.0.0.5", "172.16.4.9", "169.254.1.1"]
doc_example = "203.0.113.42"
ipv6_doc = "2001:db8::dead:beef"
"#,
    ),
    (
        "a phone-number validator's fixtures",
        r#"
const VALID: &[&str] = &["555-0100", "555-0142", "(555) 018-7000"];
const INVALID: &[&str] = &["", "abc", "1"];
"#,
    ),
    (
        "ordinary application code",
        r#"
pub fn reconcile(items: &[Item], budget: u64) -> Vec<Plan> {
    let mut remaining = budget;
    items
        .iter()
        .filter(|item| item.enabled && item.price <= remaining)
        .map(|item| {
            remaining = remaining.saturating_sub(item.price);
            Plan::new(item.id, item.price)
        })
        .collect()
}
"#,
    ),
    (
        "a lockfile-ish blob of hashes",
        r#"
checksum = "d975f5c8e0a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c5d6e7f8091a2b3c"
integrity = "sha512-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789+/=="
revision = "4685961ab8c1d2e3f405162738495a6b7c8d9e0f"
"#,
    ),
    (
        "a UUID-heavy fixture",
        r#"
let ids = [
    "36afe797-0000-4444-8888-aaaaaaaaaaaa",
    "01998f6e-0000-7000-8000-000000000000",
];
"#,
    ),
    (
        "prose with no sensitive content",
        "The reconciliation pass runs after every write and is idempotent. \
         It reads the ledger, compares it against the projection, and emits a \
         plan describing what must change.",
    ),
];

#[test]
fn ordinary_source_produces_no_substitutions() {
    let detector = Detector::new(&deterministic_tiers());
    let mut failures = Vec::new();

    for (name, fixture) in CLEAN_FIXTURES {
        let findings = detector.find(fixture);
        if !findings.is_empty() {
            let matched: Vec<&str> = findings.iter().map(|f| &fixture[f.range.clone()]).collect();
            failures.push(format!("{name}: {matched:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "the filter would have rewritten code that is not sensitive, which \
         makes the model produce something broken and the user blame the \
         model:\n  {}",
        failures.join("\n  ")
    );
}

/// The other half: things that genuinely are secrets must still be caught, or
/// the corpus test above would be trivially satisfiable by matching nothing.
const SECRET_FIXTURES: &[(&str, &str)] = &[
    (
        "github token",
        concat!("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789"),
    ),
    (
        "github fine-grained PAT",
        concat!(
            "github_pat_",
            "11ABCDEFG0123456789012_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456"
        ),
    ),
    // AWS's documentation example, assembled so repository credential guards
    // can distinguish the fixture from an accidentally committed credential.
    ("aws access key", concat!("AKIA", "IOSFODNN7EXAMPLE")),
    (
        "stripe key",
        concat!("sk_live_", "abcdefghijklmnopqrstuvwx"),
    ),
    (
        "google api key",
        concat!("AIza", "SyA1234567890abcdefghijklmnopqrstuv"),
    ),
    (
        "slack token",
        concat!("xoxb-", "123456789012-abcdefghijklmnop"),
    ),
    (
        "private key header",
        concat!("-----BEGIN ", "RSA PRIVATE KEY-----\nMIIEow..."),
    ),
];

/// A bare hex blob is a hash; the same blob labelled as a token is a secret.
#[test]
fn a_hex_blob_counts_only_when_something_calls_it_a_credential() {
    let detector = Detector::new(&deterministic_tiers());
    let blob = "d975f5c8e0a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c5d6e7f8091a2b3c";

    assert!(
        detector.find(&format!("checksum = \"{blob}\"")).is_empty(),
        "a lockfile checksum was treated as a secret; the model would rewrite \
         the lockfile around a placeholder and the build would break"
    );
    assert!(
        detector.find(&format!("revision = \"{blob}\"")).is_empty(),
        "a git revision was treated as a secret"
    );
    assert_eq!(
        detector.find(&format!("api_token = \"{blob}\"")).len(),
        1,
        "the same blob, labelled a token, must be caught"
    );
    assert_eq!(
        detector
            .find(&format!("Authorization: Bearer {blob}"))
            .len(),
        1,
        "a bearer credential must be caught"
    );
}

#[test]
fn genuine_secrets_are_still_caught() {
    let detector = Detector::new(&deterministic_tiers());
    let mut missed = Vec::new();

    for (name, fixture) in SECRET_FIXTURES {
        let text = format!("config value:\n  key = \"{fixture}\"\n");
        if detector.find(&text).is_empty() {
            missed.push(*name);
        }
    }

    assert!(
        missed.is_empty(),
        "the corpus test above would be trivially satisfiable if these were \
         missed too. Not caught: {missed:?}"
    );
}

#[test]
fn a_clean_file_is_returned_completely_unchanged() {
    // Not merely "no findings" — the substituter must produce an identical
    // document, so a request that matched nothing is byte-identical upstream.
    let detector = Detector::new(&deterministic_tiers());
    for (name, fixture) in CLEAN_FIXTURES {
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": *fixture}]
        });
        let result = ironwire_privacy::substitute(
            &detector,
            &Salt::fixed(1),
            &ironwire_privacy::Exemptions::default(),
            &body,
        );
        assert_eq!(result.body, body, "{name} was altered");
        assert_eq!(result.substitutions, 0, "{name} produced substitutions");
    }
}

/// Everything a coding agent sees constantly that must survive `pii` intact.
///
/// A false positive here is not a small annoyance: the model rewrites a config
/// or a test around a placeholder, the thing stops working, and nothing in the
/// failure points at IronWire.
const PII_CLEAN_FIXTURES: &[(&str, &str)] = &[
    ("documentation email", "Contact user@example.com for help."),
    ("rfc 2606 domains", "admin@example.org and dev@example.net"),
    ("loopback v4", "let addr = \"127.0.0.1:8463\";"),
    ("loopback v6", "bind(\"::1\", port)?;"),
    ("rfc 1918 ten", "gateway = 10.0.0.1"),
    ("rfc 1918 192", "router at 192.168.1.1"),
    ("rfc 1918 172", "docker bridge 172.16.0.1"),
    (
        "rfc 5737",
        "TEST-NET-1 is 192.0.2.0/24, TEST-NET-2 198.51.100.0/24",
    ),
    ("rfc 3849 v6", "prefix 2001:db8::1 is for documentation"),
    ("nanp fictional", "call 555-0100 or (555) 010-0199"),
    ("link local", "169.254.169.254 is the metadata endpoint"),
    // The shapes most likely to be mistaken for a phone number.
    (
        "version string",
        "ironwire 0.1.0, rustc 1.96.0, edition 2024",
    ),
    ("port range", "ports 8000-8080 and 30000-32767 are open"),
    ("git sha", "commit 3420fe1dc83722a0fa7fdb4f6f2f86b47e23b117"),
    ("timestamp", "started_at 1786218306513 and 1700000000"),
    ("semver range", ">=1.2.3, <2.0.0"),
    ("uuid", "019fe2e3-ba3b-70b2-a441-b40ab1161074"),
    ("mac address", "aa:bb:cc:dd:ee:ff"),
    ("time", "took 01:23:45 to run"),
    ("decimal numbers", "0.05 of 1.0, or 15.5 per million"),
];

fn pii_tiers() -> Tiers {
    Tiers {
        secrets: true,
        named_values: Vec::new(),
        pii: true,
    }
}

#[test]
fn realistic_code_survives_pii_mode_untouched() {
    let detector = Detector::new(&pii_tiers());
    let mut failures = Vec::new();

    for (name, fixture) in PII_CLEAN_FIXTURES {
        let findings = detector.find(fixture);
        if !findings.is_empty() {
            let matched: Vec<&str> = findings.iter().map(|f| &fixture[f.range.clone()]).collect();
            failures.push(format!("{name}: {matched:?}"));
        }
    }
    // The tier-1 corpus must also survive the higher mode unchanged.
    for (name, fixture) in CLEAN_FIXTURES {
        let findings = detector.find(fixture);
        if !findings.is_empty() {
            let matched: Vec<&str> = findings.iter().map(|f| &fixture[f.range.clone()]).collect();
            failures.push(format!("{name} (tier 1 corpus): {matched:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "`pii` would have rewritten things that identify nobody:\n  {}",
        failures.join("\n  ")
    );
}

/// The other half, so the test above cannot be satisfied by a detector that
/// matches nothing.
#[test]
fn pii_mode_catches_each_class_it_claims() {
    let detector = Detector::new(&pii_tiers());
    let cases: &[(&str, &str, Class)] = &[
        (
            "email",
            "mail alice.smith@acme-holdings.co.uk about it",
            Class::Email,
        ),
        (
            "public v4",
            "upstream 203.0.114.9 refused",
            Class::IpAddress,
        ),
        (
            "public v6",
            "peer 2606:4700:4700::1111 answered",
            Class::IpAddress,
        ),
        ("e164 phone", "ring +442071838750 tomorrow", Class::Phone),
        ("nanp phone", "ring (415) 273-9164 tomorrow", Class::Phone),
    ];

    for (name, text, class) in cases {
        let findings = detector.find(text);
        assert!(
            findings.iter().any(|f| f.class == *class),
            "`pii` found nothing of class {class:?} in the {name} case: {:?}",
            findings
                .iter()
                .map(|f| &text[f.range.clone()])
                .collect::<Vec<_>>()
        );
    }
}

/// Reserved for the tier-3 classifier, and a regex must not be allowed to
/// borrow it: a name detector that reads as protection while missing most
/// names is worse than none (`docs/PRIVACY.md` §2).
#[test]
fn the_personal_class_is_never_emitted() {
    let detector = Detector::new(&pii_tiers());
    let text = "Alice Smith and Bob Jones reviewed it with Dr. Carol Nguyen.";
    assert!(
        detector
            .find(text)
            .iter()
            .all(|f| f.class != Class::Personal),
        "a regex claimed to have found a person"
    );
}

/// `credentials` must behave exactly as the old booleans did, so an upgraded
/// install filters what it filtered before and no more.
#[test]
fn credentials_mode_leaves_pii_alone() {
    let detector = Detector::new(&deterministic_tiers());
    for (name, text, _) in &[
        (
            "email",
            "mail alice.smith@acme-holdings.co.uk",
            Class::Email,
        ),
        ("ip", "upstream 203.0.114.9 refused", Class::IpAddress),
        ("phone", "ring +442071838750", Class::Phone),
    ] {
        assert!(
            detector.find(text).is_empty(),
            "`credentials` substituted a {name}, which is `pii`'s job"
        );
    }
}
