//! Folders the sandboxed chooser hands back, as the host sees them.
//!
//! Inside a Flatpak sandbox, a folder picked in the file chooser outside the
//! app's own grants comes back as a document-portal path,
//! `$XDG_RUNTIME_DIR/doc/<id>/<name>`. That path means something only to the
//! document portal. A host daemon (`daemon::host_link`) opening a session
//! there would start the tool in the portal's FUSE mount, not in the
//! contributor's folder. So before such a path is sent to a host daemon it
//! is mapped back with `org.freedesktop.portal.Documents.GetHostPaths`
//! (interface version 5 and later), on the session bus the app already has,
//! through the `zbus` this crate already uses for the Background portal.
//! When the mapping fails the launch is refused rather than sent with the
//! portal path.

use std::path::{Path, PathBuf};

/// The document portal's mount, from a value of `XDG_RUNTIME_DIR`.
pub fn doc_root_from(xdg_runtime_dir: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let base = PathBuf::from(xdg_runtime_dir?);
    base.is_absolute().then(|| base.join("doc"))
}

/// The document id and whatever follows the exported name, when `path` is
/// inside the document portal's mount. `None` for any other path.
pub fn split_doc_path(path: &Path, doc_root: &Path) -> Option<(String, PathBuf)> {
    use std::path::Component;
    let mut parts = path.strip_prefix(doc_root).ok()?.components();
    let id = match parts.next()? {
        Component::Normal(id) => id.to_str()?.to_string(),
        _ => return None,
    };
    // The exported file's own name: required, and not part of the answer,
    // since the host path names it already.
    let Component::Normal(_) = parts.next()? else {
        return None;
    };
    let rest = parts.as_path();
    if rest
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some((id, rest.to_path_buf()))
}

/// A `GetHostPaths` value: a byte string, NUL-terminated as GLib sends it.
/// `None` unless it is a non-empty absolute path.
pub fn decode_host_path(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = bytes.strip_suffix(b"\0").unwrap_or(bytes);
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    path.is_absolute().then_some(path)
}

/// The mapping could not be made; the folder must not be sent.
#[derive(Debug, PartialEq, Eq)]
pub struct Unmapped;

/// `path` as the host sees it. Unchanged outside the portal's mount, or when
/// the daemon is not on the host (a daemon in the sandbox sees the portal
/// path itself). Inside it, the host path `lookup` gives for its document
/// id, with anything below the exported folder kept.
pub fn host_path_with(
    path: &Path,
    daemon_on_host: bool,
    doc_root: Option<&Path>,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<PathBuf, Unmapped> {
    let split = doc_root.and_then(|root| split_doc_path(path, root));
    let (true, Some((id, rest))) = (daemon_on_host, split) else {
        return Ok(path.to_path_buf());
    };
    let host = lookup(&id).ok_or(Unmapped)?;
    Ok(if rest.as_os_str().is_empty() {
        host
    } else {
        host.join(rest)
    })
}

/// Whether [`host_path`] would ask the portal: only for a path inside its
/// mount, sent to a daemon on the host.
pub fn needs_mapping(path: &Path, daemon_on_host: bool) -> bool {
    let root = doc_root_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref());
    daemon_on_host && root.is_some_and(|root| split_doc_path(path, &root).is_some())
}

/// [`host_path_with`] against the real portal. Blocking: one D-Bus round
/// trip, so callers run it off the GTK thread.
pub fn host_path(path: &Path, daemon_on_host: bool) -> Result<PathBuf, Unmapped> {
    let root = doc_root_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref());
    host_path_with(path, daemon_on_host, root.as_deref(), portal_lookup)
}

/// The host folder the portal gives for document `id`, or `None`.
fn portal_lookup(id: &str) -> Option<PathBuf> {
    // Bounded, as `portal::spawn_request` bounds its call: a bus that
    // never answers is a refusal, not a launch window that waits forever.
    let (tx, rx) = std::sync::mpsc::channel();
    let id = id.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(get_host_path(&id).ok().flatten());
    });
    rx.recv_timeout(PORTAL_TIMEOUT).ok().flatten()
}

/// The folders a `set_settings` declaration (`{"<tool>_source": {"mode":
/// "watch", "path": ...}}`) would watch, in order.
pub fn watched_paths(params: &serde_json::Value) -> Vec<PathBuf> {
    params
        .as_object()
        .into_iter()
        .flat_map(|fields| fields.values())
        .filter(|value| value["mode"] == "watch")
        .filter_map(|value| value["path"].as_str().map(PathBuf::from))
        .collect()
}

/// Whether any folder `params` declares lies in the portal's mount at
/// `doc_root`, and so must be mapped before it goes to a daemon on the host.
pub fn declarations_need_mapping_in(
    params: &serde_json::Value,
    daemon_on_host: bool,
    doc_root: Option<&Path>,
) -> bool {
    let Some(root) = doc_root.filter(|_| daemon_on_host) else {
        return false;
    };
    watched_paths(params)
        .iter()
        .any(|path| split_doc_path(path, root).is_some())
}

/// [`declarations_need_mapping_in`] for this process's portal mount.
pub fn declarations_need_mapping(params: &serde_json::Value, daemon_on_host: bool) -> bool {
    let root = doc_root_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref());
    declarations_need_mapping_in(params, daemon_on_host, root.as_deref())
}

/// `params` as a daemon on the host must be sent it: each watched folder in
/// the portal's mount at `doc_root` replaced by the host folder `lookup`
/// gives for its document id. `None`, and nothing may be sent, when one
/// cannot be mapped, its mapping is not UTF-8, or any folder left in the
/// declaration is still in the portal's mount.
pub fn host_declarations_with(
    mut params: serde_json::Value,
    doc_root: Option<&Path>,
    mut lookup: impl FnMut(&str) -> Option<PathBuf>,
) -> Option<serde_json::Value> {
    for value in params.as_object_mut()?.values_mut() {
        if value["mode"] != "watch" {
            continue;
        }
        let path = PathBuf::from(value["path"].as_str()?);
        let host = host_path_with(&path, true, doc_root, &mut lookup).ok()?;
        value["path"] = serde_json::Value::from(host.to_str()?);
    }
    // Whatever the declaration still names must be outside the portal's
    // mount: a mapping that lands back in it, or a folder in some other
    // shape, is not sent.
    let still_in_portal = |value: &serde_json::Value| {
        value["path"]
            .as_str()
            .zip(doc_root)
            .is_some_and(|(path, root)| split_doc_path(Path::new(path), root).is_some())
    };
    if params.as_object()?.values().any(still_in_portal) {
        return None;
    }
    Some(params)
}

/// [`host_declarations_with`] against the real portal. Blocking: one bounded
/// D-Bus round trip per folder, so callers run it off the GTK thread.
pub fn host_declarations(params: serde_json::Value) -> Option<serde_json::Value> {
    let root = doc_root_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref());
    host_declarations_with(params, root.as_deref(), portal_lookup)
}

/// The first `org.freedesktop.portal.Documents` interface version with
/// `GetHostPaths`.
pub const HOST_PATHS_VERSION: u32 = 5;

/// Whether a document portal reporting interface `version` has no
/// `GetHostPaths`, so no folder it exports can be mapped to the host. An
/// unknown version (no answer, or no portal) is not taken as old: the
/// caller then says only that the folder was not saved.
pub fn lacks_host_paths(version: Option<u32>) -> bool {
    version.is_some_and(|version| version < HOST_PATHS_VERSION)
}

/// The settings key (`claude_source`, ...) of the one folder `params`
/// watches inside the portal's mount at `doc_root`. `None` when there is
/// none, or more than one: a terminal opened in one folder declares one.
pub fn sole_portal_declaration_in(
    params: &serde_json::Value,
    doc_root: Option<&Path>,
) -> Option<String> {
    let root = doc_root?;
    let mut inside = params
        .as_object()?
        .iter()
        .filter(|(_, value)| value["mode"] == "watch")
        .filter(|(_, value)| {
            value["path"]
                .as_str()
                .is_some_and(|path| split_doc_path(Path::new(path), root).is_some())
        })
        .map(|(key, _)| key.clone());
    let key = inside.next()?;
    inside.next().is_none().then_some(key)
}

/// The settings key of the one portal folder in `params` that this
/// system's portal cannot map because it predates `GetHostPaths`; `None`
/// for any other refusal. Blocking: one bounded D-Bus property read, so
/// callers run it off the GTK thread.
pub fn old_portal_refusal(params: &serde_json::Value) -> Option<String> {
    let root = doc_root_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref());
    let key = sole_portal_declaration_in(params, root.as_deref())?;
    lacks_host_paths(portal_version()).then_some(key)
}

/// The document portal's interface version, or `None` when it does not
/// answer in [`PORTAL_TIMEOUT`].
fn portal_version() -> Option<u32> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(get_portal_version().ok());
    });
    rx.recv_timeout(PORTAL_TIMEOUT).ok().flatten()
}

fn get_portal_version() -> anyhow::Result<u32> {
    let connection = zbus::blocking::Connection::session()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
    )?;
    Ok(proxy.get_property::<u32>("version")?)
}

/// How long a `GetHostPaths` answer may take.
const PORTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn get_host_path(id: &str) -> anyhow::Result<Option<PathBuf>> {
    let connection = zbus::blocking::Connection::session()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
    )?;
    let paths: std::collections::HashMap<String, Vec<u8>> =
        proxy.call("GetHostPaths", &(vec![id],))?;
    Ok(paths.get(id).and_then(|bytes| decode_host_path(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    /// Version 4 (GNOME 46's portal) has no `GetHostPaths`; 5 and later
    /// do; an unknown version is not called old.
    #[test]
    fn only_a_portal_below_version_5_lacks_host_paths() {
        assert!(lacks_host_paths(Some(4)));
        assert!(lacks_host_paths(Some(1)));
        assert!(!lacks_host_paths(Some(HOST_PATHS_VERSION)));
        assert!(!lacks_host_paths(Some(6)));
        assert!(!lacks_host_paths(None));
    }

    #[test]
    fn the_sole_portal_folder_is_named_by_its_settings_key() {
        use serde_json::json;
        let root = Path::new("/run/user/1000/doc");
        let portal = |id: &str| format!("/run/user/1000/doc/{id}/work");
        let one = json!({
            "claude_source": {"mode": "watch", "path": portal("a1")},
            "codex_source": {"mode": "watch", "path": "/home/u/.codex/sessions"},
            "gemini_source": {"mode": "off"},
        });
        assert_eq!(
            sole_portal_declaration_in(&one, Some(root)).as_deref(),
            Some("claude_source")
        );
        // Two portal folders: one terminal cannot declare both.
        let two = json!({
            "claude_source": {"mode": "watch", "path": portal("a1")},
            "codex_source": {"mode": "watch", "path": portal("b2")},
        });
        assert_eq!(sole_portal_declaration_in(&two, Some(root)), None);
        // None in the mount, or no mount at all.
        let none = json!({"claude_source": {"mode": "watch", "path": "/home/u/x"}});
        assert_eq!(sole_portal_declaration_in(&none, Some(root)), None);
        assert_eq!(sole_portal_declaration_in(&one, None), None);
        // Off is never a folder, whatever it carries.
        let off = json!({"claude_source": {"mode": "off", "path": portal("a1")}});
        assert_eq!(sole_portal_declaration_in(&off, Some(root)), None);
    }

    #[test]
    fn the_portal_mount_is_doc_under_the_runtime_directory() {
        assert_eq!(
            doc_root_from(Some(OsStr::new("/run/user/1000"))),
            Some(PathBuf::from("/run/user/1000/doc"))
        );
        assert_eq!(doc_root_from(None), None);
        assert_eq!(doc_root_from(Some(OsStr::new(""))), None);
        assert_eq!(doc_root_from(Some(OsStr::new("run/user/1000"))), None);
    }

    #[test]
    fn a_portal_path_splits_into_its_id_and_what_lies_below_the_export() {
        let root = Path::new("/run/user/1000/doc");
        assert_eq!(
            split_doc_path(Path::new("/run/user/1000/doc/a1b2c3d4/project"), root),
            Some(("a1b2c3d4".to_string(), PathBuf::new()))
        );
        assert_eq!(
            split_doc_path(
                Path::new("/run/user/1000/doc/a1b2c3d4/project/src/bin"),
                root
            ),
            Some(("a1b2c3d4".to_string(), PathBuf::from("src/bin")))
        );
        for other in [
            "/home/u/project",
            "/run/user/1000/doc",
            "/run/user/1000/doc/a1b2c3d4",
            "/run/user/1000/document/a1b2c3d4/project",
            "/run/user/1000/doc/../x/project",
        ] {
            assert_eq!(split_doc_path(Path::new(other), root), None, "{other}");
        }
    }

    #[test]
    fn a_host_path_is_the_byte_string_without_its_nul() {
        assert_eq!(
            decode_host_path(b"/home/u/project\0"),
            Some(PathBuf::from("/home/u/project"))
        );
        assert_eq!(
            decode_host_path(b"/home/u/project"),
            Some(PathBuf::from("/home/u/project"))
        );
        assert_eq!(decode_host_path(b"\0"), None);
        assert_eq!(decode_host_path(b""), None);
        assert_eq!(decode_host_path(b"relative/path\0"), None);
    }

    #[test]
    fn a_portal_folder_sent_to_the_host_daemon_is_mapped_to_its_host_path() {
        let root = Path::new("/run/user/1000/doc");
        let mapped = host_path_with(
            Path::new("/run/user/1000/doc/a1b2c3d4/project/src"),
            true,
            Some(root),
            |id| {
                assert_eq!(id, "a1b2c3d4");
                Some(PathBuf::from("/home/u/work/project"))
            },
        );
        assert_eq!(mapped, Ok(PathBuf::from("/home/u/work/project/src")));
    }

    #[test]
    fn a_portal_folder_the_portal_cannot_map_is_refused() {
        let root = Path::new("/run/user/1000/doc");
        assert_eq!(
            host_path_with(
                Path::new("/run/user/1000/doc/a1b2c3d4/project"),
                true,
                Some(root),
                |_| None,
            ),
            Err(Unmapped)
        );
    }

    #[test]
    fn other_folders_and_a_sandboxed_daemon_are_left_alone() {
        let root = Path::new("/run/user/1000/doc");
        let never = |_: &str| -> Option<PathBuf> { panic!("no portal call") };
        assert_eq!(
            host_path_with(Path::new("/home/u/project"), true, Some(root), never),
            Ok(PathBuf::from("/home/u/project"))
        );
        // The daemon in the sandbox opens the portal path itself.
        assert_eq!(
            host_path_with(
                Path::new("/run/user/1000/doc/a1b2c3d4/project"),
                false,
                Some(root),
                never,
            ),
            Ok(PathBuf::from("/run/user/1000/doc/a1b2c3d4/project"))
        );
        assert_eq!(
            host_path_with(Path::new("/home/u/project"), true, None, never),
            Ok(PathBuf::from("/home/u/project"))
        );
    }

    /// A folder picked or dropped in the sandbox outside the app's grants
    /// is a document-portal path; the host daemon is sent the folder it
    /// stands for, as Launch does, and an Off answer or an ordinary path is
    /// sent as it is.
    #[test]
    fn a_portal_folder_declared_to_the_host_daemon_is_sent_as_its_host_path() {
        use serde_json::json;
        let root = Path::new("/run/user/1000/doc");
        let params = json!({
            "claude_source": {"mode": "watch", "path": "/run/user/1000/doc/a1b2/projects"},
            "codex_source": {"mode": "off"},
            "gemini_source": {"mode": "watch", "path": "/home/u/.gemini/tmp"},
        });
        assert_eq!(
            watched_paths(&params),
            vec![
                PathBuf::from("/run/user/1000/doc/a1b2/projects"),
                PathBuf::from("/home/u/.gemini/tmp")
            ]
        );
        assert!(declarations_need_mapping_in(&params, true, Some(root)));
        // A daemon in the sandbox opens the portal path itself.
        assert!(!declarations_need_mapping_in(&params, false, Some(root)));
        let mapped = host_declarations_with(params, Some(root), |id| {
            assert_eq!(id, "a1b2");
            Some(PathBuf::from("/home/u/.claude/projects"))
        });
        assert_eq!(
            mapped,
            Some(json!({
                "claude_source": {"mode": "watch", "path": "/home/u/.claude/projects"},
                "codex_source": {"mode": "off"},
                "gemini_source": {"mode": "watch", "path": "/home/u/.gemini/tmp"},
            }))
        );
    }

    /// Ordinary folders need no portal call and are sent unchanged.
    #[test]
    fn ordinary_folders_need_no_mapping() {
        use serde_json::json;
        let root = Path::new("/run/user/1000/doc");
        let params = json!({
            "claude_source": {"mode": "watch", "path": "/home/u/.claude/projects"},
            "codex_source": {"mode": "off"},
        });
        assert!(!declarations_need_mapping_in(&params, true, Some(root)));
        assert_eq!(
            host_declarations_with(params.clone(), Some(root), |_| panic!("no portal call")),
            Some(params)
        );
    }

    /// Nothing is sent when the portal cannot say where a folder is, names
    /// one that is not UTF-8, or names one still inside its own mount: the
    /// host daemon would otherwise watch the portal path as an empty mount,
    /// or a mangled path.
    #[test]
    fn a_portal_folder_that_cannot_be_mapped_is_not_declared() {
        use serde_json::json;
        use std::os::unix::ffi::OsStrExt;
        let root = Path::new("/run/user/1000/doc");
        let params = json!({
            "claude_source": {"mode": "watch", "path": "/run/user/1000/doc/a1b2/projects"},
            "codex_source": {"mode": "off"},
        });
        assert_eq!(
            host_declarations_with(params.clone(), Some(root), |_| None),
            None
        );
        assert_eq!(
            host_declarations_with(params.clone(), Some(root), |_| {
                Some(PathBuf::from(OsStr::from_bytes(b"/home/u/\xff")))
            }),
            None
        );
        assert_eq!(
            host_declarations_with(params, Some(root), |_| {
                Some(PathBuf::from("/run/user/1000/doc/c3d4/projects"))
            }),
            None
        );
        // A declaration that is not an object is not sent either.
        assert_eq!(
            host_declarations_with(json!([]), Some(root), |_| None),
            None
        );
    }
}
