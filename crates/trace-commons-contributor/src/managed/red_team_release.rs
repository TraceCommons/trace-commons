//! Pinned signed `rts` release archive installed into `<config>/red-team/runtime`.

/// A pinned, signed `rts` release archive (URL and 32-byte SHA-256 digest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleasePin {
    pub url: &'static str,
    pub sha256: [u8; 32],
}

impl ReleasePin {
    #[cfg(target_os = "macos")]
    pub fn to_archive(self) -> rts::setup::ReleaseArchive {
        rts::setup::ReleaseArchive {
            url: self.url.to_owned(),
            sha256: self.sha256,
        }
    }
}

/// Pinned `nearai/nearai-red-sandbox` release archive (`v0.1.5`, aarch64-apple-darwin).
/// SHA-256 verified against the GitHub release asset digest (`aa5ad39c...`).
pub const RTS_RELEASE: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.5/rts-0.1.5-aarch64-apple-darwin.tar.gz",
    sha256: [
        0xaa, 0x5a, 0xd3, 0x9c, 0xc0, 0x93, 0xb6, 0xc3, 0x82, 0x10, 0x54, 0x18, 0xd7, 0x8f, 0xfa,
        0x38, 0x52, 0x1b, 0xfa, 0x80, 0x48, 0x30, 0x48, 0xa1, 0x99, 0x48, 0x34, 0xf6, 0xc7, 0xb6,
        0x2a, 0x2d,
    ],
};
