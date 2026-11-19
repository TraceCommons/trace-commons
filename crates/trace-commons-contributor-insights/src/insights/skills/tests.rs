use super::*;
use std::fs;
use std::path::Path;

fn doc(name: &str) -> String {
    format!("---\nname: {name}\ndescription: A useful skill\n---\nInstructions\n")
}

fn put(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn parses_yaml_quotes_and_block_descriptions() {
    assert_eq!(parse_skill_document(b"---\r\nname: 'one''s skill'\r\ndescription: >-\r\n  First line\r\n  second line.\r\n---\r\nbody"), Some(("one's skill".into(), "First line second line.".into())));
    assert_eq!(
        parse_skill_document(b"---\nname: \"quoted\"\ndescription: \"A colon: and # text\"\n---\n"),
        Some(("quoted".into(), "A colon: and # text".into()))
    );
    assert_eq!(
        parse_skill_document(b"---\nname: block\ndescription: |\n  First\n  second\n---\n"),
        Some(("block".into(), "First\nsecond".into()))
    );
    assert_eq!(
        parse_skill_document(b"---\nname: multiline\ndescription: \"First\n  second\"\n---\n"),
        Some(("multiline".into(), "First second".into()))
    );
}

#[test]
fn rejects_missing_unclosed_and_oversized_frontmatter() {
    assert!(parse_skill_document(b"name: absent").is_none());
    assert!(parse_skill_document(b"---\nname: x\ndescription: y").is_none());
    assert!(parse_skill_document(b"---\nname: [x]\ndescription: y\n---").is_none());
    assert!(parse_skill_document(&vec![b'x'; MAX_SKILL_BYTES as usize + 1]).is_none());
}

#[test]
fn scans_only_known_directories_and_reports_invalid_documents() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".codex/skills/local/SKILL.md",
        doc("local").as_bytes(),
    );
    put(
        home.path(),
        ".config/opencode/skills/open/SKILL.md",
        doc("open").as_bytes(),
    );
    put(
        home.path(),
        "private/unrelated/SKILL.md",
        doc("ignored").as_bytes(),
    );
    put(
        project.path(),
        ".claude/skills/project/SKILL.md",
        doc("project").as_bytes(),
    );
    put(
        project.path(),
        ".claude/skills/broken/SKILL.md",
        b"not a skill",
    );
    let inventory = scan_inventory_with_roots(home.path(), &[project.path().into()], &[]);
    assert_eq!(inventory.skills.len(), 4);
    assert!(inventory.partial);
    assert!(
        inventory
            .skills
            .iter()
            .all(|s| !s.location.contains(home.path().to_str().unwrap()))
    );
    let local = inventory.skills.iter().find(|s| s.name == "local").unwrap();
    assert_eq!(local.sha256, hash_bytes(doc("local").as_bytes()));
    assert!(local.source.is_none());
}

#[cfg(unix)]
#[test]
fn symlinked_skills_keep_agent_coverage_and_lock_provenance() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".agents/skills/renamed/SKILL.md",
        doc("display-name").as_bytes(),
    );
    put(home.path(), ".agents/.skill-lock.json", br#"{"version":3,"skills":{"renamed":{"source":"publisher/repository","sourceType":"github","skillPath":"skills/original/SKILL.md"}}}"#);
    fs::create_dir_all(home.path().join(".codex/skills")).unwrap();
    symlink(
        home.path().join(".agents/skills/renamed"),
        home.path().join(".codex/skills/alias"),
    )
    .unwrap();
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    let codex = inventory
        .skills
        .iter()
        .find(|s| s.agent == "codex")
        .unwrap();
    assert_eq!(
        codex.source.as_ref().unwrap().repository,
        "publisher/repository"
    );
    assert_eq!(
        codex.source.as_ref().unwrap().path,
        "skills/original/SKILL.md"
    );
    assert!(!inventory.partial);
}

#[test]
fn lock_without_path_cannot_identify_same_named_skill() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".agents/skills/common/SKILL.md",
        doc("common").as_bytes(),
    );
    put(
        home.path(),
        ".agents/.skill-lock.json",
        br#"{"version":1,"skills":{"common":{"source":"publisher/repo","sourceType":"github"}}}"#,
    );
    assert!(
        scan_inventory_with_roots(home.path(), &[], &[]).skills[0]
            .source
            .is_none()
    );
}

fn installed(hash: &str) -> InstalledSkill {
    InstalledSkill {
        id: "local".into(),
        name: "skill".into(),
        description: "description".into(),
        agent: "codex".into(),
        scope: "global".into(),
        location: "~/.codex/skills/skill/SKILL.md".into(),
        sha256: hash.into(),
        source: Some(SkillSource {
            repository: "publisher/repo".into(),
            path: "skills/skill/SKILL.md".into(),
            revision: None,
            baseline_sha256: None,
        }),
    }
}

fn latest(hash: &str) -> CatalogSkill {
    CatalogSkill {
        id: "remote".into(),
        publisher_id: "publisher".into(),
        name: "skill".into(),
        description: "description".into(),
        repository: "publisher/repo".into(),
        path: "skills/skill/SKILL.md".into(),
        revision: "abc".into(),
        sha256: hash.into(),
        source_url: "https://github.com/publisher/repo/blob/abc/skills/skill/SKILL.md".into(),
        instructions_url: "https://github.com/publisher/repo".into(),
    }
}

#[test]
fn freshness_requires_source_and_content_history() {
    assert_eq!(
        freshness(&installed("new"), &latest("new"), &[]),
        Freshness::Current
    );
    assert_eq!(
        freshness(&installed("old"), &latest("new"), &["old".into()]),
        Freshness::UpdateAvailable
    );
    assert_eq!(
        freshness(&installed("custom"), &latest("new"), &[]),
        Freshness::NotChecked
    );
    assert_eq!(
        freshness(&installed("custom"), &latest("new"), &["old".into()]),
        Freshness::NotChecked
    );
    let mut modified = installed("custom");
    modified.source.as_mut().unwrap().revision = Some("abc".into());
    assert_eq!(
        freshness(&modified, &latest("new"), &[]),
        Freshness::NotChecked
    );
    assert_eq!(
        freshness_with_baseline(&modified, &latest("new"), &[], Some("old")),
        Freshness::LocallyModified
    );
    let mut tracked_modified = installed("custom");
    tracked_modified.source.as_mut().unwrap().baseline_sha256 = Some("baseline".into());
    assert_eq!(
        freshness(&tracked_modified, &latest("new"), &[]),
        Freshness::LocallyModified
    );
    tracked_modified.sha256 = "old".into();
    assert_eq!(
        freshness(&tracked_modified, &latest("new"), &["old".into()]),
        Freshness::LocallyModified
    );
    tracked_modified.sha256 = "new".into();
    assert_eq!(
        freshness(&tracked_modified, &latest("new"), &[]),
        Freshness::Current
    );
    let mut other = latest("new");
    other.repository = "other/repo".into();
    assert_eq!(
        freshness(&installed("new"), &other, &[]),
        Freshness::UnknownSource
    );
    let mut unknown = installed("new");
    unknown.source = None;
    assert_eq!(
        freshness(&unknown, &latest("new"), &[]),
        Freshness::UnknownSource
    );
}

#[test]
fn repository_normalization_rejects_credentials_and_traversal() {
    assert_eq!(
        normalize_repository("git@github.com:Owner/Repo.git"),
        Some("owner/repo".into())
    );
    assert_eq!(
        normalize_repository("https://github.com/Owner/Repo.git"),
        Some("owner/repo".into())
    );
    assert!(normalize_repository("https://token@github.com/owner/repo").is_none());
    assert!(normalize_repository("https://example.com/owner/repo").is_none());
    assert!(normalize_skill_path("../secrets/SKILL.md").is_none());
}

#[test]
fn oversized_file_marks_coverage_partial() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".claude/skills/huge/SKILL.md",
        &vec![b'x'; MAX_SKILL_BYTES as usize + 1],
    );
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert!(inventory.partial);
    assert_eq!(inventory.skills.len(), 1);
    let skill = &inventory.skills[0];
    assert_eq!(skill.name, "huge");
    assert!(skill.description.is_empty());
    assert!(skill.sha256.is_empty());
    assert!(skill.source.is_none());
    let details = &inventory.details[&skill.id];
    assert_eq!(details.definition_status, SkillDefinitionStatus::Unreadable);
    assert_eq!(details.availability, SkillAvailability::Unknown);
    assert!(
        inventory
            .coverage
            .iter()
            .any(|c| c.status == ScanStatus::Partial)
    );
}

#[test]
fn plugin_cache_uses_manifest_provenance_and_explicit_scope() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".codex/plugins/cache/market/superpowers/7.0.1/skills/brainstorming/SKILL.md",
        doc("brainstorming").as_bytes(),
    );
    put(
        home.path(),
        ".codex/plugins/cache/market/superpowers/7.0.1/.codex-plugin/plugin.json",
        br#"{"name":"superpowers","repository":"https://github.com/obra/superpowers","skills":"./skills/"}"#,
    );
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert_eq!(inventory.skills.len(), 1);
    assert_eq!(inventory.skills[0].scope, "plugin_cache");
    let source = inventory.skills[0].source.as_ref().unwrap();
    assert_eq!(source.repository, "obra/superpowers");
    assert_eq!(source.path, "skills/brainstorming/SKILL.md");
    assert!(source.revision.is_none());
}

#[test]
fn lock_identity_does_not_leak_to_unrelated_same_named_installation() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".agents/skills/common/SKILL.md",
        doc("common").as_bytes(),
    );
    put(
        home.path(),
        ".codex/skills/common/SKILL.md",
        doc("different").as_bytes(),
    );
    put(home.path(), ".agents/.skill-lock.json", br#"{"version":3,"skills":{"common":{"source":"publisher/repo","sourceType":"github","skillPath":"skills/common/SKILL.md"}}}"#);
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert!(
        inventory
            .skills
            .iter()
            .find(|s| s.agent == "codex")
            .unwrap()
            .source
            .is_none()
    );
    assert!(
        inventory
            .skills
            .iter()
            .find(|s| s.agent == "shared")
            .unwrap()
            .source
            .is_some()
    );
}

#[test]
fn git_provenance_requires_tracked_file_and_omits_revision_when_modified() {
    use std::process::Command;
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join(".codex/skills/git-skill");
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/publisher/repo.git",
    ]);
    put(&repo, "SKILL.md", doc("tracked").as_bytes());
    git(&["add", "SKILL.md"]);
    git(&["-c", "commit.gpgsign=false", "commit", "-qm", "Initial"]);
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert!(
        inventory.skills[0]
            .source
            .as_ref()
            .unwrap()
            .revision
            .is_some()
    );
    put(&repo, "SKILL.md", doc("changed").as_bytes());
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert!(
        inventory.skills[0]
            .source
            .as_ref()
            .unwrap()
            .revision
            .is_none()
    );
    assert_eq!(
        inventory.skills[0].source.as_ref().unwrap().baseline_sha256,
        Some(hash_bytes(doc("tracked").as_bytes()))
    );
    let mut catalog = latest("upstream-current");
    catalog.path = "SKILL.md".into();
    assert_eq!(
        freshness(&inventory.skills[0], &catalog, &[]),
        Freshness::LocallyModified
    );
    git(&["rm", "--cached", "SKILL.md"]);
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert!(inventory.skills[0].source.is_none());
}

#[test]
fn quoted_metadata_allows_trailing_comments_without_changing_content() {
    assert_eq!(
        parse_skill_document(
            b"---\nname: 'single' # comment\ndescription: \"Hash # kept\" # removed\n---\n"
        ),
        Some(("single".into(), "Hash # kept".into()))
    );
    assert_eq!(
        parse_skill_document(
            b"---\nname: block\ndescription: |\n  heading\n  ---\n  content\n---\n"
        ),
        Some(("block".into(), "heading\n---\ncontent".into()))
    );
}

#[test]
fn rejects_inconsistent_lock_remote_and_noncanonical_source_paths() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".agents/skills/common/SKILL.md",
        doc("common").as_bytes(),
    );
    put(home.path(), ".agents/.skill-lock.json", br#"{"version":3,"skills":{"common":{"source":"publisher/repo","sourceUrl":"https://github.com/other/repo.git","sourceType":"github","skillPath":"skills/common/SKILL.md"}}}"#);
    assert!(
        scan_inventory_with_roots(home.path(), &[], &[]).skills[0]
            .source
            .is_none()
    );
    assert!(normalize_skill_path("skills/./common/SKILL.md").is_none());
    assert!(normalize_skill_path("skills//common/SKILL.md").is_none());
}

#[test]
fn rejects_unbounded_display_metadata() {
    assert!(parse_skill_document(doc(&"n".repeat(MAX_SKILL_NAME_BYTES + 1)).as_bytes()).is_none());
    let description = format!(
        "---\nname: bounded\ndescription: {}\n---\n",
        "d".repeat(MAX_SKILL_DESCRIPTION_BYTES + 1)
    );
    assert!(parse_skill_document(description.as_bytes()).is_none());
}

#[cfg(unix)]
#[test]
fn never_policy_blocks_global_skill_alias_and_root_alias() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let denied = tempfile::tempdir().unwrap();
    put(
        denied.path(),
        ".agents/skills/company/SKILL.md",
        doc("private-company").as_bytes(),
    );
    fs::create_dir_all(home.path().join(".agents/skills")).unwrap();
    symlink(
        denied.path().join(".agents/skills/company"),
        home.path().join(".agents/skills/company"),
    )
    .unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    symlink(
        denied.path().join(".agents/skills"),
        home.path().join(".claude/skills"),
    )
    .unwrap();
    let inventory = scan_inventory_with_policy(home.path(), &[], &[], &[denied.path().into()]);
    assert!(inventory.skills.is_empty());
    assert!(inventory.partial);
    assert!(
        inventory
            .coverage
            .iter()
            .any(|entry| entry.status == ScanStatus::Partial)
    );
}

#[cfg(unix)]
#[test]
fn never_policy_skips_lock_and_plugin_manifest_aliases() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let denied = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".agents/skills/company/SKILL.md",
        doc("company").as_bytes(),
    );
    put(denied.path(), "lock.json", br#"{"version":3,"skills":{"company":{"source":"private/repo","sourceType":"github","skillPath":"skills/company/SKILL.md"}}}"#);
    symlink(
        denied.path().join("lock.json"),
        home.path().join(".agents/.skill-lock.json"),
    )
    .unwrap();
    let plugin = ".codex/plugins/cache/market/plugin/1.0.0";
    put(
        home.path(),
        &format!("{plugin}/skills/example/SKILL.md"),
        doc("example").as_bytes(),
    );
    put(
        denied.path(),
        "manifest.json",
        br#"{"repository":"https://github.com/private/repo"}"#,
    );
    fs::create_dir_all(home.path().join(plugin).join(".codex-plugin")).unwrap();
    symlink(
        denied.path().join("manifest.json"),
        home.path().join(plugin).join(".codex-plugin/plugin.json"),
    )
    .unwrap();
    let inventory = scan_inventory_with_policy(home.path(), &[], &[], &[denied.path().into()]);
    assert_eq!(inventory.skills.len(), 2);
    assert!(inventory.skills.iter().all(|skill| skill.source.is_none()));
    assert!(inventory.partial);
}

#[cfg(unix)]
#[test]
fn never_policy_blocks_plugin_cache_directory_alias() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let denied = tempfile::tempdir().unwrap();
    put(
        denied.path(),
        "cache/market/plugin/1.0.0/skills/private/SKILL.md",
        doc("private").as_bytes(),
    );
    fs::create_dir_all(home.path().join(".codex/plugins")).unwrap();
    symlink(
        denied.path().join("cache"),
        home.path().join(".codex/plugins/cache"),
    )
    .unwrap();
    let inventory = scan_inventory_with_policy(home.path(), &[], &[], &[denied.path().into()]);
    assert!(inventory.skills.is_empty());
    assert!(inventory.partial);
}

#[cfg(unix)]
#[test]
fn never_policy_skips_git_metadata_alias() {
    use std::os::unix::fs::symlink;
    use std::process::Command;
    let home = tempfile::tempdir().unwrap();
    let denied = tempfile::tempdir().unwrap();
    let repo = home.path().join(".agents/skills/tracked");
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/private/repo.git",
    ]);
    put(&repo, "SKILL.md", doc("tracked").as_bytes());
    git(&["add", "SKILL.md"]);
    git(&["-c", "commit.gpgsign=false", "commit", "-qm", "Initial"]);
    fs::rename(repo.join(".git"), denied.path().join("git-metadata")).unwrap();
    symlink(denied.path().join("git-metadata"), repo.join(".git")).unwrap();
    let inventory = scan_inventory_with_policy(home.path(), &[], &[], &[denied.path().into()]);
    assert_eq!(inventory.skills.len(), 1);
    assert!(inventory.skills[0].source.is_none());
    fs::remove_file(repo.join(".git")).unwrap();
    fs::write(
        repo.join(".git"),
        format!("gitdir: {}\n", denied.path().join("git-metadata").display()),
    )
    .unwrap();
    let inventory = scan_inventory_with_policy(home.path(), &[], &[], &[denied.path().into()]);
    assert_eq!(inventory.skills.len(), 1);
    assert!(inventory.skills[0].source.is_none());
}

#[test]
fn plugin_monorepo_requires_explicit_repository_directory() {
    let home = tempfile::tempdir().unwrap();
    let package = ".codex/plugins/cache/market/pstack/1.0.0";
    put(
        home.path(),
        &format!("{package}/skills/example/SKILL.md"),
        doc("example").as_bytes(),
    );
    put(
        home.path(),
        &format!("{package}/.codex-plugin/plugin.json"),
        br#"{"name":"pstack","repository":"https://github.com/cursor/plugins"}"#,
    );
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert!(inventory.skills[0].source.is_none());
    put(home.path(), &format!("{package}/.codex-plugin/plugin.json"), br#"{"name":"pstack","repository":{"url":"https://github.com/cursor/plugins","directory":"pstack"}}"#);
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert_eq!(
        inventory.skills[0].source.as_ref().unwrap().path,
        "pstack/skills/example/SKILL.md"
    );
    put(home.path(), &format!("{package}/.codex-plugin/plugin.json"), br#"{"name":"pstack","repository":{"url":"https://github.com/cursor/plugins","directory":"../private"}}"#);
    assert!(
        scan_inventory_with_roots(home.path(), &[], &[]).skills[0]
            .source
            .is_none()
    );
}

#[test]
fn unresolved_never_policy_refuses_local_scan() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".agents/skills/local/SKILL.md",
        doc("local").as_bytes(),
    );
    let inventory = scan_inventory_with_policy(
        home.path(),
        &[],
        &[],
        &[home.path().join("missing-never-root")],
    );
    assert!(inventory.skills.is_empty());
    assert!(inventory.partial);
}

#[cfg(unix)]
#[test]
fn never_policy_blocks_verified_plugin_manifest_content() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let denied = tempfile::tempdir().unwrap();
    let package = ".codex/plugins/cache/market/superpowers/7.0.1";
    put(
        home.path(),
        &format!("{package}/skills/example/SKILL.md"),
        doc("example").as_bytes(),
    );
    put(
        denied.path(),
        "manifest.json",
        br#"{"name":"superpowers","repository":"https://github.com/obra/superpowers"}"#,
    );
    fs::create_dir_all(home.path().join(package).join(".codex-plugin")).unwrap();
    symlink(
        denied.path().join("manifest.json"),
        home.path().join(package).join(".codex-plugin/plugin.json"),
    )
    .unwrap();
    assert!(
        scan_inventory_with_roots(home.path(), &[], &[]).skills[0]
            .source
            .is_some()
    );
    let inventory = scan_inventory_with_policy(home.path(), &[], &[], &[denied.path().into()]);
    assert_eq!(inventory.skills.len(), 1);
    assert!(inventory.skills[0].source.is_none());
    assert!(inventory.partial);
}

#[test]
fn malformed_inventory_entry_is_retained_with_invalid_status() {
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".codex/skills/broken/SKILL.md",
        b"not frontmatter",
    );
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert_eq!(inventory.skills.len(), 1);
    let skill = &inventory.skills[0];
    assert_eq!(
        inventory.details[&skill.id].definition_status,
        SkillDefinitionStatus::Invalid
    );
    assert_eq!(
        inventory.details[&skill.id].availability,
        SkillAvailability::Unknown
    );
    let json = serde_json::to_string(&inventory).unwrap();
    assert!(!json.contains(home.path().to_str().unwrap()));
}

#[cfg(unix)]
#[test]
fn same_agent_aliases_deduplicate_and_keep_alias_locations() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".codex/skills/real/SKILL.md",
        doc("same").as_bytes(),
    );
    symlink(
        home.path().join(".codex/skills/real"),
        home.path().join(".codex/skills/alias"),
    )
    .unwrap();
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert_eq!(inventory.skills.len(), 1);
    assert_eq!(inventory.details[&inventory.skills[0].id].aliases.len(), 2);
}

#[test]
fn claude_registry_selects_active_plugin_version_and_disabled_state() {
    let home = tempfile::tempdir().unwrap();
    for (name, version) in [("enabled", "1"), ("enabled", "2"), ("disabled", "1")] {
        put(
            home.path(),
            &format!(".claude/plugins/cache/vendor/{name}/{version}/skills/example/SKILL.md"),
            doc(name).as_bytes(),
        );
    }
    put(
        home.path(),
        ".claude/settings.json",
        br#"{"enabledPlugins":{"enabled@vendor":true,"disabled@vendor":false}}"#,
    );
    let registry = serde_json::json!({"version":2,"plugins":{
        "enabled@vendor":[{"scope":"user","installPath":home.path().join(".claude/plugins/cache/vendor/enabled/2")}],
        "disabled@vendor":[{"scope":"user","installPath":home.path().join(".claude/plugins/cache/vendor/disabled/1")}]
    }});
    put(
        home.path(),
        ".claude/plugins/installed_plugins.json",
        &serde_json::to_vec(&registry).unwrap(),
    );
    let inventory = scan_inventory_with_roots(home.path(), &[], &[]);
    assert_eq!(inventory.skills.len(), 3);
    let states: Vec<_> = inventory
        .skills
        .iter()
        .map(|skill| inventory.details[&skill.id].availability)
        .collect();
    assert!(states.contains(&SkillAvailability::Available));
    assert!(states.contains(&SkillAvailability::Disabled));
    assert!(states.contains(&SkillAvailability::Cached));
}

#[test]
fn configured_agent_root_replaces_global_default_but_keeps_indexed_projects() {
    let home = tempfile::tempdir().unwrap();
    let alternate = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    put(
        home.path(),
        ".codex/skills/stale/SKILL.md",
        doc("stale-default").as_bytes(),
    );
    put(
        alternate.path(),
        "skills/current/SKILL.md",
        doc("configured").as_bytes(),
    );
    put(
        project.path(),
        ".codex/skills/project/SKILL.md",
        doc("project").as_bytes(),
    );
    let inventory = scan_inventory_with_roots(
        home.path(),
        &[project.path().into()],
        &[("codex".into(), alternate.path().join("skills"))],
    );
    let names: Vec<_> = inventory
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"configured"));
    assert!(names.contains(&"project"));
    assert!(!names.contains(&"stale-default"));
}

#[cfg(unix)]
#[test]
fn never_alias_under_symlinked_root_is_not_lost_during_listing() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    put(
        target.path(),
        "skills/private/SKILL.md",
        doc("private").as_bytes(),
    );
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    symlink(
        target.path().join("skills"),
        home.path().join(".codex/skills"),
    )
    .unwrap();
    let inventory = scan_inventory_with_policy(
        home.path(),
        &[],
        &[],
        &[home.path().join(".codex/skills/private")],
    );
    assert!(inventory.skills.is_empty());
    assert!(inventory.partial);
}

#[test]
fn native_skill_paths_use_repository_separators_without_relaxing_external_paths() {
    let expected = Some("skills/example/SKILL.md".to_string());
    let native = Path::new("skills").join("example").join("SKILL.md");
    assert_eq!(provenance::native_skill_path(&native), expected);
    assert_eq!(
        provenance::native_skill_path(Path::new("skills/example/SKILL.md")),
        expected
    );
    assert!(provenance::native_skill_path(Path::new("../skills/example/SKILL.md")).is_none());
    assert!(provenance::native_skill_path(Path::new("/skills/example/SKILL.md")).is_none());
    // External repository paths remain strict on every OS: a backslash is
    // never accepted as a separator in a manifest or lockfile identity.
    assert!(normalize_skill_path(r"skills\example\SKILL.md").is_none());
    #[cfg(windows)]
    assert_eq!(
        provenance::native_skill_path(Path::new(r"skills\example\SKILL.md")),
        expected
    );
    // On Unix these backslashes are literal filename bytes, not separators.
    #[cfg(unix)]
    assert!(provenance::native_skill_path(Path::new(r"skills\example\SKILL.md")).is_none());
}
