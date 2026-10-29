//! Reading Codex's compressed rollouts (`rollout-<...>.jsonl.zst`).
//!
//! Codex compresses a rollout in place once it is a few days old: the
//! `.jsonl` is replaced by a `.jsonl.zst` of the same name with the same
//! last-write time. Only the Insights counter pass reads these
//! ([`super::TraceSource::discover_insights_only`]); nothing here is
//! offered for contribution or uploaded.
//!
//! The reader streams: it never decompresses to memory whole or to a temp
//! file, and it refuses a frame whose window would make the decoder itself
//! large. Its output is bounded twice, because a few kilobytes of zstd can
//! claim gigabytes of output:
//!
//! - [`MAX_DECOMPRESSED_BYTES`] of output from one file, in all.
//! - [`MAX_DECOMPRESSION_RATIO`] times the file's own size, once the output
//!   passes [`RATIO_FLOOR_BYTES`] (small files of repeated records compress
//!   far better than large ones and are harmless).
//!
//! Crossing either ends the read with an error carrying
//! [`DecompressionLimit`], so a caller can tell a refused file from a
//! corrupt one without matching on message text.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

/// The suffix a compressed rollout adds to its plain name.
pub const COMPRESSED_SUFFIX: &str = ".zst";

/// The most bytes one compressed file may decompress to. Above the largest
/// plain rollout measured on a heavy Codex user's machine (453 MB), so a
/// compressed session is read at least as far as its plain twin would be;
/// 13 of 5,457 compressed rollouts on that machine were larger than 64 MiB
/// decompressed, and 2 larger than this.
pub const MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// The most output per byte of input, checked once the output passes
/// [`RATIO_FLOOR_BYTES`]. The highest ratio measured over 5,457 real
/// rollouts was 22.
pub const MAX_DECOMPRESSION_RATIO: u64 = 64;

/// Output below this is never refused for its ratio.
pub const RATIO_FLOOR_BYTES: u64 = 16 * 1024 * 1024;

/// The largest decoder window accepted, as a log2: 16 MiB. Codex's own
/// frames use 2 MiB or less; the decoder's default would allow 128 MiB of
/// window memory for a frame that merely claims it.
pub const WINDOW_LOG_MAX: u32 = 24;

/// Codex's frames record their decompressed size. When one does not, the
/// size is estimated as this many times the file's own (4.3 was the mean
/// measured), for budgeting reads only.
pub const ESTIMATED_RATIO: u64 = 5;

/// Whether `path` names a compressed rollout by its suffix.
pub fn is_compressed(path: &Path) -> bool {
    path.as_os_str()
        .to_str()
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
}

/// A compressed file's output passed a bound. Carried inside the
/// [`io::Error`] the reader returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("compressed session output exceeds its bound")]
pub struct DecompressionLimit;

/// Why a compressed file could not be read to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressedFailure {
    /// Not zstd, truncated, or corrupt.
    Undecodable,
    /// Its output passed [`MAX_DECOMPRESSED_BYTES`] or
    /// [`MAX_DECOMPRESSION_RATIO`].
    TooLarge,
}

impl CompressedFailure {
    /// The failure an error from [`BoundedDecoder`] stands for.
    pub fn of(error: &io::Error) -> Self {
        if error
            .get_ref()
            .is_some_and(|inner| inner.is::<DecompressionLimit>())
        {
            CompressedFailure::TooLarge
        } else {
            CompressedFailure::Undecodable
        }
    }
}

/// A streaming zstd decoder over one file, bounded as the module describes.
/// The first error it returns is remembered ([`BoundedDecoder::failure`]),
/// so a caller whose own parser swallows read errors can still say why the
/// read ended.
pub struct BoundedDecoder {
    inner: zstd::stream::read::Decoder<'static, BufReader<File>>,
    compressed_bytes: u64,
    produced: u64,
    failure: Option<CompressedFailure>,
}

impl BoundedDecoder {
    /// Open `path` for decompression.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let compressed_bytes = file.metadata()?.len();
        Self::from_file(file, compressed_bytes)
    }

    fn from_file(file: File, compressed_bytes: u64) -> io::Result<Self> {
        let mut inner = zstd::stream::read::Decoder::new(file)?;
        inner.window_log_max(WINDOW_LOG_MAX)?;
        Ok(Self {
            inner,
            compressed_bytes,
            produced: 0,
            failure: None,
        })
    }

    /// Why the read ended early, if it did.
    pub fn failure(&self) -> Option<CompressedFailure> {
        self.failure
    }

    /// Bytes of output so far.
    pub fn produced(&self) -> u64 {
        self.produced
    }

    fn over_limit(&self) -> bool {
        self.produced > MAX_DECOMPRESSED_BYTES
            || (self.produced > RATIO_FLOOR_BYTES
                && self.produced
                    > self
                        .compressed_bytes
                        .saturating_mul(MAX_DECOMPRESSION_RATIO))
    }
}

impl Read for BoundedDecoder {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(match self.failure {
                Some(CompressedFailure::TooLarge) => io::Error::other(DecompressionLimit),
                _ => io::Error::other("compressed session undecodable"),
            });
        }
        match self.inner.read(buf) {
            Ok(read) => {
                self.produced = self.produced.saturating_add(read as u64);
                if self.over_limit() {
                    self.failure = Some(CompressedFailure::TooLarge);
                    return Err(io::Error::other(DecompressionLimit));
                }
                Ok(read)
            }
            Err(error) => {
                self.failure = Some(CompressedFailure::of(&error));
                Err(error)
            }
        }
    }
}

/// The decompressed size `path`'s first frame records, read from its
/// header without decompressing; else [`ESTIMATED_RATIO`] times the file's
/// own size. For budgeting reads, never for bounding them.
pub fn estimated_decompressed_bytes(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let compressed = file.metadata().ok()?.len();
    // A frame header is at most 18 bytes.
    let mut header = [0u8; 18];
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => break,
        }
    }
    let declared = zstd::zstd_safe::get_frame_content_size(&header[..filled])
        .ok()
        .flatten();
    Some(declared.unwrap_or_else(|| compressed.saturating_mul(ESTIMATED_RATIO)))
}

/// A session file's lines, decompressed when it is a compressed rollout.
pub fn open_lines(path: &Path) -> io::Result<Box<dyn BufRead>> {
    if is_compressed(path) {
        Ok(Box::new(BufReader::new(BoundedDecoder::open(path)?)))
    } else {
        Ok(Box::new(BufReader::new(File::open(path)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn decodes_what_was_encoded_and_reports_its_declared_size() {
        let dir = tempfile::tempdir().unwrap();
        let plain = b"{\"type\":\"session_meta\"}\n{\"type\":\"event_msg\"}\n".repeat(50);
        let path = write(
            dir.path(),
            "rollout-a.jsonl.zst",
            &zstd::encode_all(&plain[..], 3).unwrap(),
        );
        let mut out = Vec::new();
        let mut decoder = BoundedDecoder::open(&path).unwrap();
        decoder.read_to_end(&mut out).unwrap();
        assert_eq!(out, plain);
        assert_eq!(decoder.failure(), None);
        // A streamed frame records no size: estimated from the file's own.
        let compressed = std::fs::metadata(&path).unwrap().len();
        assert_eq!(
            estimated_decompressed_bytes(&path),
            Some(compressed * ESTIMATED_RATIO)
        );
        // A one-shot frame, as Codex writes, records it.
        let path = write(
            dir.path(),
            "rollout-a2.jsonl.zst",
            &zstd::bulk::compress(&plain, 3).unwrap(),
        );
        assert_eq!(
            estimated_decompressed_bytes(&path),
            Some(plain.len() as u64)
        );
    }

    #[test]
    fn a_corrupt_file_is_undecodable_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = zstd::encode_all(&b"{\"a\":1}\n".repeat(1000)[..], 3).unwrap();
        let cut = bytes.len() / 2;
        bytes.truncate(cut);
        let path = write(dir.path(), "rollout-b.jsonl.zst", &bytes);
        let mut decoder = BoundedDecoder::open(&path).unwrap();
        let error = decoder.read_to_end(&mut Vec::new()).unwrap_err();
        assert_eq!(
            CompressedFailure::of(&error),
            CompressedFailure::Undecodable
        );
        assert_eq!(decoder.failure(), Some(CompressedFailure::Undecodable));

        let path = write(dir.path(), "rollout-c.jsonl.zst", b"not zstd at all\n");
        let mut decoder = BoundedDecoder::open(&path).unwrap();
        assert!(decoder.read_to_end(&mut Vec::new()).is_err());
        assert_eq!(decoder.failure(), Some(CompressedFailure::Undecodable));
    }

    #[test]
    fn output_past_the_ratio_bound_stops_the_read_with_bounded_memory() {
        let dir = tempfile::tempdir().unwrap();
        // 40 MiB of one byte compresses to a few KiB: far over the ratio.
        let bomb = zstd::encode_all(std::io::repeat(b'x').take(40 * 1024 * 1024), 19).unwrap();
        assert!((bomb.len() as u64) * MAX_DECOMPRESSION_RATIO < RATIO_FLOOR_BYTES);
        let path = write(dir.path(), "rollout-d.jsonl.zst", &bomb);
        let mut decoder = BoundedDecoder::open(&path).unwrap();
        let mut sink = [0u8; 64 * 1024];
        let error = loop {
            match decoder.read(&mut sink) {
                Ok(0) => panic!("the bomb decompressed to its end"),
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert_eq!(CompressedFailure::of(&error), CompressedFailure::TooLarge);
        assert_eq!(decoder.failure(), Some(CompressedFailure::TooLarge));
        // Stopped within one read of the floor.
        assert!(decoder.produced() <= RATIO_FLOOR_BYTES + sink.len() as u64);
    }

    #[test]
    fn plain_files_open_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "rollout-e.jsonl", b"one\ntwo\n");
        let lines: Vec<String> = open_lines(&path)
            .unwrap()
            .lines()
            .map(Result::unwrap)
            .collect();
        assert_eq!(lines, ["one", "two"]);
        assert!(!is_compressed(&path));
        assert!(is_compressed(Path::new("x/rollout-e.jsonl.zst")));
    }
}
