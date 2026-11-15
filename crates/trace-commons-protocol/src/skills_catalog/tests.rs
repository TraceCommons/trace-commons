use super::*;
use chrono::{Duration, TimeZone};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use serde_json::{Value, json};

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}

/// A throwaway key for this test run only. Never a real catalog key.
fn key() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn public(key: &Ed25519KeyPair) -> [u8; 32] {
    key.public_key().as_ref().try_into().unwrap()
}

fn sign_with(key: &Ed25519KeyPair, domain_input: Vec<u8>, payload: &str) -> Vec<u8> {
    assemble_signed_catalog(
        payload.into(),
        vec![CatalogSignature {
            key_id: key_id_for(&public(key)),
            signature: hex::encode(key.sign(&domain_input).as_ref()),
        }],
    )
}

fn sign(key: &Ed25519KeyPair, payload: &str) -> Vec<u8> {
    sign_with(key, signing_input(payload.as_bytes()), payload)
}

const REV: &str = "0123456789abcdef0123456789abcdef01234567";
const HASH: &str = "aa00000000000000000000000000000000000000000000000000000000000000";
const OLD_HASH: &str = "bb00000000000000000000000000000000000000000000000000000000000000";

fn raw() -> Value {
    let issued = now();
    json!({
        "schema_version": 1,
        "checks_version": 1,
        "environment": "staging",
        "revision": issued.timestamp(),
        "issued_at": issued,
        "expires_at": issued + Duration::days(14),
        "publishers": [
            {
                "id": "gh.acme.skills",
                "name": "acme/skills",
                "repository": "acme/skills",
                "url": "https://github.com/acme/skills",
                "instructions_url": "https://github.com/acme/skills",
                "source_kind": "marketplace",
                "signals": {"stars": 12, "pushed_at": issued - Duration::days(3),
                            "created_at": issued - Duration::days(300), "archived": false,
                            "commits_7d": 1, "commits_28d": 4,
                            "checked_at": issued - Duration::hours(1)}
            },
            {
                "id": "obra",
                "name": "Obra",
                "repository": "obra/superpowers",
                "url": "https://github.com/obra/superpowers",
                "instructions_url": "https://github.com/obra/superpowers#installation",
                "skills_root": "skills",
                "source_kind": "seed",
                "signals": {"checked_at": issued - Duration::hours(1)}
            }
        ],
        "skills": [
            skill("gh.acme.skills", "acme/skills", "plugins/a/skills/lint/SKILL.md"),
            skill("obra", "obra/superpowers", "skills/brainstorming/SKILL.md"),
        ]
    })
}

fn skill(publisher: &str, repository: &str, path: &str) -> Value {
    json!({
        "id": format!("{publisher}:{path}"),
        "publisher_id": publisher,
        "name": "lint",
        "description": "Checks things.\nTwice.",
        "repository": repository,
        "path": path,
        "revision": REV,
        "sha256": HASH,
        "previous_sha256": [OLD_HASH],
        "source_url": format!("https://github.com/{repository}/blob/{REV}/{path}"),
        "license": {"status": "declared", "spdx": "MIT"},
        "first_seen_at": now() - Duration::days(30),
        "last_changed_at": now() - Duration::days(2),
        "checks": {
            "frontmatter": "valid",
            "document_bytes": 2048,
            "directory_bytes": 4096,
            "directory_files": 2,
            "bundled": {"scripts": 1, "executables": 0, "binaries": 0},
            "patterns": ["pipe_to_shell"],
            "inactive": false
        }
    })
}

fn decode(value: &Value) -> Result<CatalogDocument, CatalogError> {
    decode_payload(
        serde_json::to_string(value).unwrap().as_bytes(),
        Environment::Staging,
    )
}

#[test]
fn a_well_formed_catalog_decodes() {
    let document = decode(&raw()).unwrap();
    assert_eq!(document.skills.len(), 2);
    assert_eq!(document.publishers[1].source_kind, SourceKind::Seed);
    assert!(document.is_current(now()));
}

#[test]
fn signed_catalog_round_trips_under_a_pinned_key_only() {
    let signer = key();
    let payload = serde_json::to_string(&raw()).unwrap();
    let served = sign(&signer, &payload);
    let (document, digest) =
        parse_signed_catalog(&served, &[public(&signer)], Environment::Staging, now()).unwrap();
    assert_eq!(document.revision, now().timestamp() as u64);
    assert_eq!(digest.len(), 64);
    // Another key is not trusted, and no key trusts nothing.
    assert_eq!(
        parse_signed_catalog(&served, &[public(&key())], Environment::Staging, now())
            .unwrap_err()
            .0,
        "skills-catalog-signature"
    );
    assert_eq!(
        parse_signed_catalog(&served, &[], Environment::Staging, now())
            .unwrap_err()
            .0,
        "skills-catalog-no-trusted-key"
    );
}

#[test]
fn a_feature_manifest_signature_is_not_a_catalog_signature() {
    let signer = key();
    let payload = serde_json::to_string(&raw()).unwrap();
    let flag_signed = sign_with(
        &signer,
        crate::feature_flags::signing_input(payload.as_bytes()),
        &payload,
    );
    assert_eq!(
        open_signed_catalog(&flag_signed, &[public(&signer)])
            .unwrap_err()
            .0,
        "skills-catalog-signature"
    );
    let bare = sign_with(&signer, payload.as_bytes().to_vec(), &payload);
    assert!(open_signed_catalog(&bare, &[public(&signer)]).is_err());
}

#[test]
fn signature_count_and_size_are_bounded() {
    let signer = key();
    let payload = serde_json::to_string(&raw()).unwrap();
    let one = CatalogSignature {
        key_id: key_id_for(&public(&signer)),
        signature: hex::encode(signer.sign(&signing_input(payload.as_bytes())).as_ref()),
    };
    let five = assemble_signed_catalog(payload.clone(), vec![one.clone(); 5]);
    assert_eq!(
        open_signed_catalog(&five, &[public(&signer)])
            .unwrap_err()
            .0,
        "skills-catalog-signature-count"
    );
    let none = assemble_signed_catalog(payload, vec![]);
    assert!(open_signed_catalog(&none, &[public(&signer)]).is_err());
    let huge = vec![b' '; MAX_CATALOG_BYTES + 1];
    assert_eq!(
        open_signed_catalog(&huge, &[public(&signer)])
            .unwrap_err()
            .0,
        "skills-catalog-too-large"
    );
}

#[test]
fn time_is_checked_separately_from_structure() {
    let document = decode(&raw()).unwrap();
    assert!(!document.is_current(now() + Duration::days(15)));
    assert!(document.is_current(now() - Duration::minutes(4)));
    assert!(!document.is_current(now() - Duration::minutes(6)));
}

fn refused(mutate: impl FnOnce(&mut Value)) -> &'static str {
    let mut value = raw();
    mutate(&mut value);
    decode(&value).unwrap_err().0
}

#[test]
fn document_identity_rules() {
    assert_eq!(
        refused(|v| v["schema_version"] = json!(2)),
        "skills-catalog-identity"
    );
    assert_eq!(
        refused(|v| v["checks_version"] = json!(0)),
        "skills-catalog-identity"
    );
    assert_eq!(
        refused(|v| v["environment"] = json!("production")),
        "skills-catalog-identity"
    );
    assert_eq!(
        refused(|v| v["revision"] = json!(5)),
        "skills-catalog-revision"
    );
    assert_eq!(
        refused(|v| v["expires_at"] = json!(now() + Duration::days(31))),
        "skills-catalog-validity"
    );
    assert_eq!(
        refused(|v| v["expires_at"] = json!(now())),
        "skills-catalog-validity"
    );
    assert_eq!(
        refused(|v| v["unexpected"] = json!(true)),
        "skills-catalog-json"
    );
    assert_eq!(
        refused(|v| v["skills"][0]["checks"]["safe"] = json!(true)),
        "skills-catalog-json"
    );
}

#[test]
fn publisher_rules() {
    assert_eq!(
        refused(|v| v["publishers"][0]["id"] = json!("acme")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| v["publishers"][1]["id"] = json!("gh.obra.superpowers")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| v["publishers"][0]["url"] = json!("https://evil.example/acme/skills")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| v["publishers"][0]["instructions_url"] = json!("https://example.com")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| v["publishers"][0]["repository"] = json!("Acme/Skills")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| v["publishers"][1]["instructions_url"] = json!("http://x.example")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| v["publishers"][1]["skills_root"] = json!("../skills")),
        "skills-catalog-publisher"
    );
    assert_eq!(
        refused(|v| {
            let first = v["publishers"][0].clone();
            v["publishers"][0] = v["publishers"][1].clone();
            v["publishers"][1] = first;
        }),
        "skills-catalog-order"
    );
    assert_eq!(
        refused(|v| v["publishers"][0]["signals"]["checked_at"] = json!(now() + Duration::hours(1))),
        "skills-catalog-publisher"
    );
}

#[test]
fn skill_rules() {
    for (field, value) in [
        ("id", json!("obra:skills/other/SKILL.md")),
        ("publisher_id", json!("missing")),
        ("repository", json!("acme/other")),
        ("path", json!("plugins/../SKILL.md")),
        ("path", json!("plugins/a/README.md")),
        ("revision", json!("ABCDEF")),
        ("sha256", json!("zz")),
        ("source_url", json!("https://github.com/acme/skills")),
        ("name", json!("")),
        ("name", json!("x".repeat(MAX_NAME_BYTES + 1))),
        ("description", json!("bell\u{7}")),
        ("previous_sha256", json!([HASH])),
        (
            "previous_sha256",
            json!(vec![OLD_HASH; MAX_PREVIOUS_HASHES + 1]),
        ),
        ("license", json!({"status": "absent", "spdx": "MIT"})),
        (
            "license",
            json!({"status": "declared", "spdx": "MIT License"}),
        ),
        ("last_changed_at", json!(now() - Duration::days(31))),
    ] {
        assert_eq!(
            refused(|v| v["skills"][0][field] = value.clone()),
            "skills-catalog-skill",
            "{field}={value}"
        );
    }
    for (field, value) in [
        ("document_bytes", json!(MAX_DOCUMENT_BYTES + 1)),
        ("directory_files", json!(MAX_DIRECTORY_FILES + 1)),
        ("patterns", json!(["looks_fine"])),
        ("patterns", json!(["sudo", "pipe_to_shell"])),
    ] {
        assert_eq!(
            refused(|v| v["skills"][0]["checks"][field] = value.clone()),
            "skills-catalog-skill",
            "{field}={value}"
        );
    }
    assert_eq!(
        refused(|v| v["skills"][1] = v["skills"][0].clone()),
        "skills-catalog-order"
    );
}

#[test]
fn licence_without_an_spdx_id_is_still_declared() {
    let mut value = raw();
    value["skills"][0]["license"] = json!({"status": "declared"});
    assert!(decode(&value).is_ok());
    value["skills"][0]["license"] = json!({"status": "unknown"});
    assert!(decode(&value).is_ok());
}

#[test]
fn bounds_on_counts() {
    let mut document = decode(&raw()).unwrap();
    let template = document.skills[0].clone();
    document.skills = (0..=MAX_SKILLS)
        .map(|n| {
            let mut s = template.clone();
            s.path = format!("plugins/a/skills/s{n:05}/SKILL.md");
            s.id = format!("gh.acme.skills:{}", s.path);
            s.source_url = skill_source_url("acme/skills", REV, &s.path);
            s
        })
        .collect();
    assert_eq!(document.validate().unwrap_err().0, "skills-catalog-bounds");
    document.skills.truncate(MAX_SKILLS);
    assert!(document.validate().is_ok());
}

#[test]
fn generated_ids_and_urls() {
    assert_eq!(generated_publisher_id("acme/skills"), "gh.acme.skills");
    assert_eq!(
        skill_source_url("acme/skills", REV, "a/SKILL.md"),
        format!("https://github.com/acme/skills/blob/{REV}/a/SKILL.md")
    );
    assert_eq!(
        normalize_repository(" Acme/Skills.git "),
        Some("acme/skills".into())
    );
    assert_eq!(
        normalize_repository("https://github.com/Acme/Skills"),
        Some("acme/skills".into())
    );
    for bad in [
        "acme",
        "acme/skills/extra",
        "../x",
        "a b/c",
        "https://gitlab.com/a/b",
        "a/..",
    ] {
        assert_eq!(normalize_repository(bad), None, "{bad}");
    }
    assert!(safe_skill_path("SKILL.md"));
    assert!(safe_skill_path("skills/a_b-c.d/SKILL.md"));
    for bad in [
        "",
        "/SKILL.md",
        "a//SKILL.md",
        "a/./SKILL.md",
        "a/SKILL.MD",
        "a b/SKILL.md",
    ] {
        assert!(!safe_skill_path(bad), "{bad}");
    }
}

#[test]
fn a_suggestion_is_a_public_address_and_nothing_else() {
    let good = json!({"repository": "acme/skills", "path": "skills/x/SKILL.md", "revision": REV});
    let parsed = SkillSuggestion::parse(good.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.repository, "acme/skills");
    for (body, label) in [
        (
            json!({"repository": "acme/skills", "path": "skills/x/SKILL.md", "revision": REV, "install_id": "x"}),
            "skills-suggestion-malformed",
        ),
        (
            json!({"repository": "Acme/Skills", "path": "skills/x/SKILL.md", "revision": REV}),
            "skills-suggestion-repository",
        ),
        (
            json!({"repository": "acme/skills", "path": "/etc/SKILL.md", "revision": REV}),
            "skills-suggestion-path",
        ),
        (
            json!({"repository": "acme/skills", "path": "skills/x/SKILL.md", "revision": "HEAD"}),
            "skills-suggestion-revision",
        ),
    ] {
        assert_eq!(
            SkillSuggestion::parse(body.to_string().as_bytes()).unwrap_err(),
            label,
            "{body}"
        );
    }
    assert_eq!(
        SkillSuggestion::parse(&vec![b' '; MAX_SUGGESTION_BYTES + 1]).unwrap_err(),
        "skills-suggestion-too-large"
    );
}
