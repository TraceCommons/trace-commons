use super::{
    InstalledSkill, Inventory, MAX_PROJECT_ROOTS, MAX_SKILL_BYTES, MAX_SKILLS, Root, ScanCoverage,
    ScanStatus, SkillAvailability, SkillDefinitionStatus, SkillInstallationDetails, hash_bytes,
    parse_skill_document, policy::ReadPolicy, provenance,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_DIRECTORY_ENTRIES: usize = 8192;

/// Scan standard locations plus explicitly configured SKILLS_HOME/CODEX_HOME.
/// Callers needing a deterministic environment use `scan_inventory_with_roots`.
pub fn scan_inventory(home: &Path, projects: &[PathBuf]) -> Inventory {
    scan_inventory_with_roots(home, projects, &configured_global_roots())
}

/// Explicit agent environment locations; no commands or configuration code are executed.
pub fn configured_global_roots() -> Vec<(String, PathBuf)> {
    let mut extra = Vec::new();
    if let Some(path) = std::env::var_os("SKILLS_HOME").filter(|p| !p.is_empty()) {
        extra.push(("shared".into(), PathBuf::from(path)));
    }
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|p| !p.is_empty()) {
        extra.push(("codex".into(), PathBuf::from(path).join("skills")));
    }
    for (variable, agent, suffix) in [
        ("CLAUDE_CONFIG_DIR", "claude_code", "skills"),
        ("OPENCODE_CONFIG_DIR", "opencode", "skills"),
        ("XDG_CONFIG_HOME", "opencode", "opencode/skills"),
    ] {
        if let Some(path) = std::env::var_os(variable).filter(|p| !p.is_empty()) {
            extra.push((agent.into(), PathBuf::from(path).join(suffix)));
        }
    }
    extra
}

/// Read direct skill children of known roots only, with one explicit `.system`
/// group for Codex. Symlinks may target installed bundles outside those roots;
/// only their exact SKILL.md is canonicalized/read, never the destination tree.
/// Additional roots are supplied configuration, not discovered project folders.
pub fn scan_inventory_with_roots(
    home: &Path,
    projects: &[PathBuf],
    extra_global_roots: &[(String, PathBuf)],
) -> Inventory {
    scan_inventory_with_policy(home, projects, extra_global_roots, &[])
}

/// Apply configured Never-folder exclusions before every canonical listing or
/// document/metadata read. Invalid exclusion resolution refuses the whole scan.
/// Git subprocess provenance is disabled when exclusions are present because
/// Git can follow includes/alternates outside its repository metadata root.
pub fn scan_inventory_with_policy(
    home: &Path,
    projects: &[PathBuf],
    extra_global_roots: &[(String, PathBuf)],
    denied_roots: &[PathBuf],
) -> Inventory {
    let policy = match ReadPolicy::new(denied_roots) {
        Ok(policy) => policy,
        Err(_) => {
            return Inventory {
                partial: true,
                coverage: vec![ScanCoverage {
                    location: "read-policy".into(),
                    status: ScanStatus::Partial,
                }],
                ..Inventory::default()
            };
        }
    };
    let mut inventory = Inventory {
        partial: projects.len() > MAX_PROJECT_ROOTS || extra_global_roots.len() > MAX_PROJECT_ROOTS,
        ..Inventory::default()
    };
    let mut roots = Vec::new();
    let project_roots = &projects[..projects.len().min(MAX_PROJECT_ROOTS)];
    for (base, scope, label) in std::iter::once((home, "global", "~".to_string())).chain(
        project_roots
            .iter()
            .enumerate()
            .map(|(i, p)| (p.as_path(), "project", format!("project-{}", i + 1))),
    ) {
        for (agent, relative) in [
            ("codex", ".codex/skills"),
            ("claude_code", ".claude/skills"),
            ("opencode", ".config/opencode/skills"),
            ("opencode", ".config/opencode/skill"),
            ("opencode", ".opencode/skills"),
            ("opencode", ".opencode/skill"),
            ("shared", ".agents/skills"),
        ] {
            // An explicit agent home replaces its global default, but never
            // changes which already-indexed project roots are inspected.
            if scope == "global"
                && extra_global_roots
                    .iter()
                    .any(|(configured, _)| configured == agent)
            {
                continue;
            }
            roots.push(Root {
                path: base.join(relative),
                agent: agent.into(),
                scope: scope.into(),
                location: format!("{label}/{relative}"),
            });
        }
    }
    for (index, (agent, path)) in extra_global_roots
        .iter()
        .take(MAX_PROJECT_ROOTS)
        .enumerate()
    {
        roots.push(Root {
            path: path.clone(),
            agent: agent.clone(),
            scope: "global".into(),
            location: format!("configured-{}", index + 1),
        });
    }
    // Explicit roots win duplicate aliases before fallback locations.
    roots.sort_by_key(|root| !root.location.starts_with("configured-"));
    let claude_config = extra_global_roots
        .iter()
        .find(|(agent, _)| agent == "claude_code")
        .and_then(|(_, skills)| skills.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".claude"));
    let plugin_states = claude_plugin_roots(&claude_config, &mut roots, &mut inventory, &policy);
    let mut budget = MAX_DIRECTORY_ENTRIES;
    let deadline = Instant::now() + Duration::from_secs(10);
    // Cache metadata gets its own bounded budget and a shorter deadline so a
    // large cache cannot consume the capacity reserved for actual skill roots.
    let mut plugin_budget = MAX_DIRECTORY_ENTRIES;
    plugin_roots(
        home,
        extra_global_roots,
        &mut roots,
        &mut plugin_budget,
        &mut inventory,
        deadline.min(Instant::now() + Duration::from_secs(2)),
        &policy,
    );
    let (sources, partial_provenance) =
        provenance::lock_sources(home, project_roots, deadline, &policy);
    if partial_provenance {
        inventory.partial = true;
        inventory.coverage.push(ScanCoverage {
            location: "source-metadata".into(),
            status: ScanStatus::Partial,
        });
    }
    for root in roots {
        // Visit bounded aliases too: document-level dedup retains their
        // locations while preserving each agent view of shared installations.
        if let Err(error) = policy.canonical_allowed(&root.path) {
            record_read_error(&mut inventory, &root.location, &error);
            continue;
        }
        scan_root(
            &root,
            &sources,
            &mut inventory,
            &mut budget,
            deadline,
            &policy,
        );
        if root.agent == "codex" {
            scan_root(
                &Root {
                    path: root.path.join(".system"),
                    location: format!("{}/.system", root.location),
                    agent: root.agent.clone(),
                    scope: root.scope.clone(),
                },
                &sources,
                &mut inventory,
                &mut budget,
                deadline,
                &policy,
            );
        }
    }
    for details in inventory.details.values_mut() {
        if let Some((_, availability)) = plugin_states
            .iter()
            .find(|(root, _)| details.canonical_path.starts_with(root))
        {
            if details.definition_status == SkillDefinitionStatus::Valid {
                details.availability = *availability;
            }
        }
    }
    inventory
        .skills
        .sort_by(|a, b| a.location.cmp(&b.location).then(a.agent.cmp(&b.agent)));
    inventory
}

fn scan_root(
    root: &Root,
    sources: &std::collections::BTreeMap<PathBuf, super::SkillSource>,
    inventory: &mut Inventory,
    budget: &mut usize,
    deadline: Instant,
    policy: &ReadPolicy,
) {
    let mut status = ScanStatus::Complete;
    let entries = match policy.read_dir(&root.path) {
        Ok(entries) => entries,
        Err(error) => {
            let status = if error.kind() == std::io::ErrorKind::NotFound {
                ScanStatus::Missing
            } else {
                inventory.partial = true;
                ScanStatus::Partial
            };
            inventory.coverage.push(ScanCoverage {
                location: root.location.clone(),
                status,
            });
            return;
        }
    };
    for entry in entries {
        if *budget == 0 || inventory.skills.len() >= MAX_SKILLS || Instant::now() >= deadline {
            status = ScanStatus::Partial;
            break;
        }
        *budget -= 1;
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                status = ScanStatus::Partial;
                continue;
            }
        };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        // No recursion: an entry must have this exact leaf document.
        // Preserve the lexical alias for Never checks as well as checking its
        // canonical target. read_dir returns entries under the canonical root.
        let document = root.path.join(entry.file_name()).join("SKILL.md");
        let canonical = match policy.canonical_allowed(&document) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                status = ScanStatus::Partial;
                continue;
            }
        };
        let (bytes, definition_status, metadata) = match policy.read(&canonical, MAX_SKILL_BYTES) {
            Ok(bytes) => {
                let metadata = parse_skill_document(&bytes);
                let state = if metadata.is_some() {
                    SkillDefinitionStatus::Valid
                } else {
                    SkillDefinitionStatus::Invalid
                };
                (bytes, state, metadata)
            }
            Err(_) => (Vec::new(), SkillDefinitionStatus::Unreadable, None),
        };
        if definition_status != SkillDefinitionStatus::Valid {
            status = ScanStatus::Partial;
        }
        let (name, description) = metadata.unwrap_or_else(|| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                String::new(),
            )
        });
        let location = format!(
            "{}/{}/SKILL.md",
            root.location,
            entry.file_name().to_string_lossy()
        );
        // Deduplicate aliases per agent and scope without conflating distinct
        // installations that merely have matching names or contents.
        if let Some(existing) = inventory.skills.iter().find(|skill| {
            skill.agent == root.agent
                && skill.scope == root.scope
                && inventory
                    .details
                    .get(&skill.id)
                    .is_some_and(|details| details.canonical_path == canonical)
        }) {
            let details = inventory
                .details
                .get_mut(&existing.id)
                .expect("known installation");
            if !details.aliases.contains(&location) {
                details.aliases.push(location);
            }
            continue;
        }
        let mut source = sources.get(&canonical).cloned();
        if source.is_none() && !policy.has_exclusions() {
            source = provenance::git_source(&canonical, &bytes);
        }
        if source.is_none() && root.scope == "plugin_cache" {
            let (plugin_source, partial_metadata) = provenance::plugin_source(&document, policy);
            source = plugin_source;
            if partial_metadata {
                status = ScanStatus::Partial;
            }
        }
        // Include the content fingerprint so replacing a path cannot silently
        // reassign retained observations to different definition bytes.
        let fingerprint = if bytes.is_empty() {
            String::new()
        } else {
            hash_bytes(&bytes)
        };
        let id = hash_bytes(
            format!(
                "{}\0{}\0{}\0{}",
                root.agent,
                root.scope,
                canonical.display(),
                fingerprint
            )
            .as_bytes(),
        );
        inventory.details.insert(
            id.clone(),
            SkillInstallationDetails {
                aliases: vec![location.clone()],
                canonical_path: canonical,
                availability: if root.scope == "plugin_cache" {
                    SkillAvailability::Cached
                } else {
                    SkillAvailability::Unknown
                },
                definition_status,
            },
        );
        inventory.skills.push(InstalledSkill {
            id,
            name,
            description,
            agent: root.agent.clone(),
            scope: root.scope.clone(),
            location,
            sha256: fingerprint,
            source,
        });
    }
    inventory.partial |= status == ScanStatus::Partial;
    if root.scope == "plugin_cache" && status == ScanStatus::Complete {
        status = ScanStatus::CachedPluginActivationUnknown;
    }
    inventory.coverage.push(ScanCoverage {
        location: root.location.clone(),
        status,
    });
}

/// Plugin caches have a fixed publisher/plugin/version/skills shape. These
/// rows explicitly carry plugin_cache scope: cache presence does not prove a
/// plugin is enabled in the active agent. No arbitrary cache tree is traversed.
fn plugin_roots(
    home: &Path,
    configured: &[(String, PathBuf)],
    roots: &mut Vec<Root>,
    budget: &mut usize,
    inventory: &mut Inventory,
    deadline: Instant,
    policy: &ReadPolicy,
) {
    let mut caches: Vec<(String, PathBuf, String)> = configured
        .iter()
        .take(MAX_PROJECT_ROOTS)
        .enumerate()
        .filter(|(_, (agent, _))| agent == "codex" || agent == "claude_code")
        .filter_map(|(index, (agent, skills))| {
            skills.parent().map(|base| {
                (
                    agent.clone(),
                    base.join("plugins/cache"),
                    format!("configured-{}/plugins/cache", index + 1),
                )
            })
        })
        .collect();
    for (agent, relative) in [
        ("codex", ".codex/plugins/cache"),
        ("claude_code", ".claude/plugins/cache"),
    ] {
        if !configured
            .iter()
            .any(|(configured_agent, _)| configured_agent == agent)
        {
            caches.push((agent.into(), home.join(relative), format!("~/{relative}")));
        }
    }
    for (agent, base, location) in caches {
        let mut frontier = vec![(base, location)];
        for _ in 0..3 {
            let mut next = Vec::new();
            for (path, location) in frontier {
                let entries = match policy.read_dir(&path) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        record_read_error(inventory, &location, &error);
                        continue;
                    }
                };
                for entry in entries {
                    if *budget == 0 || Instant::now() >= deadline {
                        inventory.partial = true;
                        return;
                    }
                    *budget -= 1;
                    let Ok(entry) = entry else {
                        inventory.partial = true;
                        continue;
                    };
                    // Cache grouping directories must be real directories;
                    // skill children may still be ordinary installation links.
                    if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                        next.push((
                            entry.path(),
                            format!("{location}/{}", entry.file_name().to_string_lossy()),
                        ));
                    }
                }
            }
            frontier = next;
        }
        for (path, location) in frontier {
            match policy.canonical_allowed(&path.join("skills")) {
                Ok(_) => roots.push(Root {
                    path: path.join("skills"),
                    agent: agent.clone(),
                    scope: "plugin_cache".into(),
                    location: format!("{location}/skills"),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => record_read_error(inventory, &format!("{location}/skills"), &error),
            }
        }
    }
}

fn record_read_error(inventory: &mut Inventory, location: &str, error: &std::io::Error) {
    let status = if error.kind() == std::io::ErrorKind::NotFound {
        ScanStatus::Missing
    } else {
        inventory.partial = true;
        ScanStatus::Partial
    };
    inventory.coverage.push(ScanCoverage {
        location: location.into(),
        status,
    });
}

/// Claude's user-scoped installation registry selects the active version.
/// enabledPlugins alone cannot establish which cached version is active.
/// Project/local entries are deliberately not treated as globally available.
fn claude_plugin_roots(
    config: &Path,
    roots: &mut Vec<Root>,
    inventory: &mut Inventory,
    policy: &ReadPolicy,
) -> Vec<(PathBuf, SkillAvailability)> {
    let mut states = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut remaining = MAX_SKILLS;
    let read_json = |path: &Path| -> std::io::Result<serde_json::Value> {
        serde_json::from_slice(&policy.read(path, super::MAX_METADATA_BYTES)?)
            .map_err(|_| std::io::Error::other("invalid_plugin_metadata"))
    };
    let settings = match read_json(&config.join("settings.json")) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return states,
        Err(error) => {
            record_read_error(inventory, "~/.claude/settings.json", &error);
            return states;
        }
    };
    let registry = match read_json(&config.join("plugins/installed_plugins.json")) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return states,
        Err(error) => {
            record_read_error(
                inventory,
                "~/.claude/plugins/installed_plugins.json",
                &error,
            );
            return states;
        }
    };
    let Some(plugins) = registry
        .get("plugins")
        .and_then(serde_json::Value::as_object)
    else {
        inventory.partial = true;
        inventory.coverage.push(ScanCoverage {
            location: "~/.claude/plugins/installed_plugins.json".into(),
            status: ScanStatus::Partial,
        });
        return states;
    };
    if plugins.len() > MAX_SKILLS {
        inventory.partial = true;
    }
    for (index, (name, installations)) in plugins.iter().take(MAX_SKILLS).enumerate() {
        let Some(installations) = installations.as_array() else {
            inventory.partial = true;
            continue;
        };
        if installations.len() > MAX_PROJECT_ROOTS {
            inventory.partial = true;
        }
        for installation in installations.iter().take(MAX_PROJECT_ROOTS) {
            if remaining == 0 || Instant::now() >= deadline {
                inventory.partial = true;
                inventory.coverage.push(ScanCoverage {
                    location: "claude-plugin-registry".into(),
                    status: ScanStatus::Partial,
                });
                return states;
            }
            remaining -= 1;
            if installation
                .get("scope")
                .and_then(serde_json::Value::as_str)
                != Some("user")
            {
                continue;
            }
            let Some(path) = installation
                .get("installPath")
                .and_then(serde_json::Value::as_str)
            else {
                inventory.partial = true;
                continue;
            };
            let path = PathBuf::from(path).join("skills");
            let location = format!("claude-plugin-{}", index + 1);
            let canonical = match policy.canonical_allowed(&path) {
                Ok(path) => path,
                Err(error) => {
                    record_read_error(inventory, &location, &error);
                    continue;
                }
            };
            let availability = match settings
                .get("enabledPlugins")
                .and_then(|plugins| plugins.get(name))
                .and_then(serde_json::Value::as_bool)
            {
                Some(true) => SkillAvailability::Available,
                Some(false) => SkillAvailability::Disabled,
                None => SkillAvailability::Unknown,
            };
            roots.push(Root {
                path,
                agent: "claude_code".into(),
                scope: "plugin_cache".into(),
                location,
            });
            states.push((canonical, availability));
        }
    }
    states
}
