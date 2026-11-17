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
    host_path_with(path, daemon_on_host, root.as_deref(), |id| {
        // Bounded, as `portal::spawn_request` bounds its call: a bus that
        // never answers is a refusal, not a launch window that waits forever.
        let (tx, rx) = std::sync::mpsc::channel();
        let id = id.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(get_host_path(&id).ok().flatten());
        });
        rx.recv_timeout(PORTAL_TIMEOUT).ok().flatten()
    })
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
}
