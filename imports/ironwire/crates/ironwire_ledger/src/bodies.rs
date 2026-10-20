//! Verbatim capture of the bodies an exchange put on the wire.
//!
//! The ledger's other columns are *observations* — numbers a provider reported,
//! which are worth having even when they are approximate. These bytes are not
//! like that. A NEAR AI receipt (`GET /v1/signature/{id}`) is an EIP-191
//! signature over the string `<sha256 of the request body as sent>:<sha256 of
//! the response body as received>`, so anything that re-serialises a body —
//! pretty-printing, reordering keys, re-escaping a non-ASCII character,
//! round-tripping a float — turns a valid receipt into a hash mismatch, which
//! reads as tampering rather than as the capture bug it is.
//!
//! So this module stores **bytes**, never a parsed value, and never a `String`:
//! a body is whatever the wire carried, including a body that is not valid
//! UTF-8. Nothing here normalises, and nothing truncates — a body we could not
//! hold whole is recorded as absent (see `ironwire_proxy::pipeline`), because
//! the digest of a truncated body is a wrong answer and the ledger's standing
//! rule is that an absent number beats a fabricated one.
//!
//! Off unless `capture.bodies = true`: these are the user's source code.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of exactly these bytes.
///
/// The digest a receipt is checked against. Takes `&[u8]` and not `&str` on
/// purpose: the moment a body becomes a `String` it has been through a lossy
/// conversion that a non-UTF-8 byte would silently change.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// [`sha256_hex`], fed in pieces.
///
/// For a body that is hashed as it streams past and never held: the digest of
/// the concatenated chunks is the digest of the whole, so the answer is the
/// same one [`sha256_hex`] gives over the same bytes, without the bytes.
#[derive(Debug, Clone, Default)]
pub struct StreamingSha256(Sha256);

impl StreamingSha256 {
    /// Start an empty digest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next chunk, in wire order.
    pub fn update(&mut self, chunk: &[u8]) {
        self.0.update(chunk);
    }

    /// Lowercase hex, exactly as [`sha256_hex`] renders it.
    #[must_use]
    pub fn finish_hex(self) -> String {
        let digest = self.0.finalize();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}

/// Bodies on disk, under `$IRONWIRE_HOME/bodies` (`docs/PACKAGING.md`).
///
/// Files rather than ledger blobs because that is what the runtime layout
/// documents and what `ironwire report` excludes by default
/// (`docs/TRUST.md` §5).
#[derive(Debug)]
pub struct BodyStore {
    dir: PathBuf,
    seq: AtomicU64,
}

impl BodyStore {
    /// Open (creating) the store at `dir`.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when the directory cannot be created.
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        restrict(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            seq: AtomicU64::new(0),
        })
    }

    /// Write one exchange's bodies and return the reference the ledger row
    /// carries.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when either file cannot be written. Callers on the
    /// response path log and carry on: a capture problem must not fail a
    /// user's inference request.
    pub fn store(&self, request: &[u8], response: &[u8]) -> io::Result<String> {
        let reference = self.next_reference();
        std::fs::write(self.dir.join(format!("{reference}.req")), request)?;
        std::fs::write(self.dir.join(format!("{reference}.res")), response)?;
        Ok(reference)
    }

    /// Read back the bodies a reference names.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when the reference is not one of ours, or either file is
    /// missing.
    pub fn read(&self, reference: &str) -> io::Result<(Vec<u8>, Vec<u8>)> {
        // The reference comes off a row we wrote, but it reaches here as a
        // string from a file on disk; a `..` in it would read whatever the
        // daemon can. Validated rather than trusted.
        if !is_reference(reference) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a body reference",
            ));
        }
        Ok((
            std::fs::read(self.dir.join(format!("{reference}.req")))?,
            std::fs::read(self.dir.join(format!("{reference}.res")))?,
        ))
    }

    /// Forget one exchange's bodies. Missing files are not an error — pruning
    /// runs repeatedly over the same rows.
    ///
    /// # Errors
    ///
    /// [`io::Error`] for anything but a missing file.
    pub fn remove(&self, reference: &str) -> io::Result<()> {
        if !is_reference(reference) {
            return Ok(());
        }
        for suffix in ["req", "res"] {
            match std::fs::remove_file(self.dir.join(format!("{reference}.{suffix}"))) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Delete every file the ledger no longer claims.
    ///
    /// Run once at daemon start, and deliberately not on the periodic prune:
    /// at start there is nothing in flight, whereas a sweep during serving
    /// could land in the gap between [`BodyStore::store`] writing the files
    /// and the row that names them being inserted, and delete a body that was
    /// about to be referenced.
    ///
    /// Returns how many files were removed.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when the directory cannot be listed. A file that cannot
    /// be removed is logged past, not fatal -- a sweep that gave up halfway
    /// would leave more behind than one that continued.
    pub fn retain_only(&self, keep: &std::collections::BTreeSet<String>) -> io::Result<usize> {
        let mut removed = 0usize;
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some((stem, suffix)) = name.rsplit_once('.') else {
                continue;
            };
            if !matches!(suffix, "req" | "res") || !is_reference(stem) || keep.contains(stem) {
                continue;
            }
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(error) => tracing::debug!(%error, "could not sweep an orphaned body"),
            }
        }
        Ok(removed)
    }

    /// Unique within this process, and ordered, so two exchanges arriving in
    /// the same nanosecond still get their own pair of files.
    fn next_reference(&self) -> String {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        format!("{nanos:020}-{seq:06}")
    }
}

/// `<digits>-<digits>`, and nothing else: no separator, no dot, no `..`.
fn is_reference(reference: &str) -> bool {
    let Some((left, right)) = reference.split_once('-') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.bytes().all(|b| b.is_ascii_digit())
        && right.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(unix)]
fn restrict(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict(_dir: &Path) -> io::Result<()> {
    // Windows inherits the ACL of `$IRONWIRE_HOME`, which the daemon already
    // creates for the current user only.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body no re-serialiser leaves alone: keys out of alphabetical order,
    /// two spaces after a colon, a non-ASCII character, a float that does not
    /// round-trip through the shortest representation, and an escaped
    /// character `serde_json` would emit differently.
    const AWKWARD: &[u8] =
        b"{\"z\":1,  \"a\":\"caf\xc3\xa9 \\u00e9\",\"f\":0.1000000000000000055511151231257827,\"m\":[]}";

    #[test]
    fn the_stored_request_is_byte_for_byte_what_was_handed_over() {
        let home = tempfile::tempdir().expect("tempdir");
        let store = BodyStore::open(home.path()).expect("store opens");
        let reference = store.store(AWKWARD, b"res").expect("stored");
        let (request, response) = store.read(&reference).expect("read back");
        assert_eq!(request, AWKWARD, "the request survived unchanged");
        assert_eq!(response, b"res");
    }

    /// The point of the whole module: re-serialising this body changes its
    /// digest, so a test that only round-tripped `{"a":1}` would pass with the
    /// bug present.
    #[test]
    fn re_serialising_the_body_would_change_its_digest() {
        let value: serde_json::Value = serde_json::from_slice(AWKWARD).expect("valid JSON");
        let reserialised = serde_json::to_vec(&value).expect("serialises");
        assert_ne!(
            reserialised.as_slice(),
            AWKWARD,
            "the fixture must actually be one a re-serialiser changes"
        );
        assert_ne!(sha256_hex(&reserialised), sha256_hex(AWKWARD));
    }

    #[test]
    fn a_body_that_is_not_utf8_is_stored_as_it_arrived() {
        let home = tempfile::tempdir().expect("tempdir");
        let store = BodyStore::open(home.path()).expect("store opens");
        let invalid = b"\xff\xfe not utf-8 at all";
        let reference = store.store(invalid, invalid).expect("stored");
        let (request, _) = store.read(&reference).expect("read back");
        assert_eq!(request, invalid);
    }

    #[test]
    fn the_digest_is_the_one_a_receipt_is_checked_against() {
        // sha256("") — the value every other implementation agrees on.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// Hashing a body in pieces must give the digest of the whole, or a
    /// streamed response would never match its receipt.
    #[test]
    fn a_streamed_digest_is_the_digest_of_the_whole() {
        let body = b"data: {\"id\":\"x\"}\n\ndata: [DONE]\n\n";
        for split in [0, 1, 7, body.len()] {
            let mut streaming = StreamingSha256::new();
            streaming.update(&body[..split]);
            streaming.update(&body[split..]);
            assert_eq!(streaming.finish_hex(), sha256_hex(body), "split at {split}");
        }
        assert_eq!(StreamingSha256::new().finish_hex(), sha256_hex(b""));
    }

    #[test]
    fn two_exchanges_in_the_same_instant_do_not_share_files() {
        let home = tempfile::tempdir().expect("tempdir");
        let store = BodyStore::open(home.path()).expect("store opens");
        let first = store.store(b"one", b"a").expect("stored");
        let second = store.store(b"two", b"b").expect("stored");
        assert_ne!(first, second);
        assert_eq!(store.read(&first).expect("read").0, b"one");
        assert_eq!(store.read(&second).expect("read").0, b"two");
    }

    #[test]
    fn a_reference_cannot_walk_out_of_the_store() {
        let home = tempfile::tempdir().expect("tempdir");
        let store = BodyStore::open(home.path()).expect("store opens");
        for hostile in ["../ledger.sqlite", "..", "a/b", "1-2/../../x", ""] {
            assert!(
                store.read(hostile).is_err(),
                "{hostile:?} should not be readable"
            );
        }
    }

    #[test]
    fn removing_bodies_twice_is_not_an_error() {
        let home = tempfile::tempdir().expect("tempdir");
        let store = BodyStore::open(home.path()).expect("store opens");
        let reference = store.store(b"one", b"a").expect("stored");
        store.remove(&reference).expect("removed");
        store.remove(&reference).expect("removing again is a no-op");
        assert!(store.read(&reference).is_err());
    }
}
