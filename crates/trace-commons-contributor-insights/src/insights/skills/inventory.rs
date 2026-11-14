use super::{
    InstalledSkill, Inventory, MAX_PROJECT_ROOTS, MAX_SKILL_BYTES, MAX_SKILLS, Root, ScanCoverage,
    ScanStatus, hash_bytes, parse_skill_document, policy::ReadPolicy, provenance,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_DIRECTORY_ENTRIES: usize = 8192;

/// Scan standard locations plus explicitly configured SKILLS_HOME/CODEX_HOME.
/// Callers needing a deterministic environment use `scan_inventory_with_roots`.
pub fn scan_inventory(home: &Path, projects: &[PathBuf]) -> Inventory {
    let mut extra = Vec::new();
    if let Some(path) = std::env::var_os("SKILLS_HOME").filter(|p| !p.is_empty()) {
        extra.push(("shared".into(), PathBuf::from(path)));
    }
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|p| !p.is_empty()) {
        extra.push(("codex".into(), PathBuf::from(path).join("skills")));
    }
    scan_inventory_with_roots(home, projects, &extra)
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
    let mut budget = MAX_DIRECTORY_ENTRIES;
    let deadline = Instant::now() + Duration::from_secs(10);
    // Cache metadata gets its own bounded budget and a shorter deadline so a
    // large cache cannot consume the capacity reserved for actual skill roots.
    let mut plugin_budget = MAX_DIRECTORY_ENTRIES;
    plugin_roots(
        home,
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
    let mut seen_roots = BTreeSet::new();
    for root in roots {
        // Preserve each agent view of shared symlinks but avoid a configured
        // alias producing two copies for the same agent and scope.
        let canonical = match policy.canonical_allowed(&root.path) {
            Ok(path) => path,
            Err(error) => {
                record_read_error(&mut inventory, &root.location, &error);
                continue;
            }
        };
        if !seen_roots.insert((root.agent.clone(), root.scope.clone(), canonical)) {
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
        let document = entry.path().join("SKILL.md");
        let canonical = match policy.canonical_allowed(&document) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                status = ScanStatus::Partial;
                continue;
            }
        };
        let Ok(bytes) = policy.read(&canonical, MAX_SKILL_BYTES) else {
            status = ScanStatus::Partial;
            continue;
        };
        let Some((name, description)) = parse_skill_document(&bytes) else {
            status = ScanStatus::Partial;
            continue;
        };
        let location = format!(
            "{}/{}/SKILL.md",
            root.location,
            entry.file_name().to_string_lossy()
        );
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
        inventory.skills.push(InstalledSkill {
            id: hash_bytes(format!("{}\0{}\0{}", root.agent, root.scope, location).as_bytes()),
            name,
            description,
            agent: root.agent.clone(),
            scope: root.scope.clone(),
            location,
            sha256: hash_bytes(&bytes),
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
    roots: &mut Vec<Root>,
    budget: &mut usize,
    inventory: &mut Inventory,
    deadline: Instant,
    policy: &ReadPolicy,
) {
    for (agent, relative) in [
        ("codex", ".codex/plugins/cache"),
        ("claude_code", ".claude/plugins/cache"),
    ] {
        let base = home.join(relative);
        let mut frontier = vec![(base, format!("~/{relative}"))];
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
                    agent: agent.into(),
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
