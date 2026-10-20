//! Writing a file without a window where it is half-written.
//!
//! `std::fs::write` truncates and then writes. A crash, a full disk, or an
//! `OOM` kill between those two leaves a truncated file — and IronWire's state
//! files are all read with a fallback, so a truncated one does not *fail*, it
//! silently reverts to defaults.
//!
//! For `consent.json` that is the worst possible shape: the reader fails closed
//! (correct), so a crash while recording one consent silently withdraws *every*
//! consent the user ever gave. Nothing about the symptom points at a crash, and
//! the user just sees IronWire forget something they explicitly authorised —
//! which is precisely the kind of thing that ends trust in a tool holding
//! credentials.
//!
//! Write to a sibling temp file, fsync it, then rename. Rename within a
//! directory is atomic on every platform IronWire targets, so a reader sees
//! either the old file or the new one and never a fragment.

use std::io::Write;
use std::path::Path;

/// Write `contents` to `path` atomically, with owner-only permissions.
///
/// # Errors
///
/// Propagates any I/O failure. A caller that could not persist must not carry
/// on as though it had.
pub fn write(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    // A sibling, so the rename stays within one filesystem. `$TMPDIR` could be
    // a different mount, and a cross-device rename is not atomic — it is a copy
    // and would reintroduce exactly the window this exists to close.
    let temp = path.with_extension(format!("tmp{}", std::process::id()));

    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        restrict(&file)?;
        file.write_all(contents.as_bytes())?;
        // Durability before visibility: without this the rename can land while
        // the contents are still in the page cache, so a power loss leaves an
        // empty file with a valid name — the same failure, one layer down.
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();

    if result.is_err() {
        // Do not leave litter behind on failure.
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Owner-only. These files record what a user authorised and which port a
/// daemon holds; another local user must not be able to rewrite them.
#[cfg(unix)]
fn restrict(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_file: &std::fs::File) -> std::io::Result<()> {
    // Windows ACLs are inherited from the containing directory, which
    // `$IRONWIRE_HOME` already restricts.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_reads_back_exactly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        write(&path, "{\"a\":1}").expect("writes");
        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "{\"a\":1}");
    }

    #[test]
    fn overwriting_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        write(&path, "first").expect("writes");
        write(&path, "second").expect("overwrites");

        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "second");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name != "state.json")
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn a_missing_parent_directory_is_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("deeper").join("state.json");
        write(&path, "ok").expect("writes");
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");
        write(&path, "{}").expect("writes");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "another local user could rewrite what this user authorised"
        );
    }

    #[test]
    fn the_temp_file_is_a_sibling_not_in_tmpdir() {
        // A cross-device rename is a copy, which is not atomic — it would
        // reintroduce the window this module exists to close. Asserted by
        // construction: the temp path must share the target's parent.
        let path = Path::new("/some/dir/consent.json");
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        assert_eq!(temp.parent(), path.parent());
    }
}
