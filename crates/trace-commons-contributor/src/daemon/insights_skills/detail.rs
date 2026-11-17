//! On-demand local definition reads. No caller-supplied path is accepted.
use super::*;

pub(super) fn handle(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req
        .params
        .get("skill_id")
        .and_then(Value::as_str)
        .filter(|id| is_hex(id, 64))
    else {
        return Response::err(req.id, ERR_BAD_PARAMS, "skills-definition-id-invalid");
    };
    let unavailable = || Response::ok(req.id, json!({"skill_id":id,"status":"unavailable"}));
    if !feature_enabled(shared) {
        return unavailable();
    }
    let Ok(policy) = inventory_policy(shared) else {
        return unavailable();
    };
    if policy.all_local_excluded {
        return unavailable();
    }
    let selected = shared
        .insights_skills
        .runtime
        .lock()
        .ok()
        .and_then(|runtime| {
            let (_, cached_policy, inventory) = runtime.inventory.as_ref()?;
            if cached_policy != &policy {
                return None;
            }
            let skill = inventory.skills.iter().find(|skill| skill.id == id)?;
            let details = inventory.details.get(id)?;
            Some((details.canonical_path.clone(), skill.sha256.clone()))
        });
    let Some((path, fingerprint)) = selected else {
        return unavailable();
    };
    let Some(definition) = read_definition(&path, &fingerprint, &policy.denied_roots) else {
        return unavailable();
    };
    if !feature_enabled(shared) || inventory_policy(shared).ok().as_ref() != Some(&policy) {
        return unavailable();
    }
    Response::ok(
        req.id,
        json!({"skill_id":id,"status":"available","definition":definition}),
    )
}

fn read_definition(path: &Path, fingerprint: &str, denied: &[PathBuf]) -> Option<String> {
    if !path.is_absolute() || path.file_name()? != "SKILL.md" {
        return None;
    }
    // Inventory retained the canonical path. A replacement symlink must not
    // redirect this capability to another file, even in a permitted root.
    if path.canonicalize().ok()? != path {
        return None;
    }
    for root in denied {
        let target = root.canonicalize().ok()?;
        if path.starts_with(root) || path.starts_with(target) {
            return None;
        }
    }
    let file = crate::evidence_import::open_import_file(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((MAX_SKILL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_SKILL_BYTES || hex::encode(Sha256::digest(&bytes)) != fingerprint {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn definition_requires_current_identity_and_current_permission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, "hello").unwrap();
        let path = path.canonicalize().unwrap();
        let fingerprint = hex::encode(Sha256::digest(b"hello"));
        assert_eq!(
            read_definition(&path, &fingerprint, &[]).as_deref(),
            Some("hello")
        );
        assert!(read_definition(&path, &fingerprint, &[dir.path().to_path_buf()]).is_none());
        std::fs::write(&path, "replacement").unwrap();
        assert!(read_definition(&path, &fingerprint, &[]).is_none());
    }
    #[cfg(unix)]
    #[test]
    fn definition_rejects_retargeted_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("other");
        std::fs::write(&target, "hello").unwrap();
        let path = dir.path().join("SKILL.md");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(read_definition(&path, &hex::encode(Sha256::digest(b"hello")), &[]).is_none());
    }
}
