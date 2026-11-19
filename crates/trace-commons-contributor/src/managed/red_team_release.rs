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

/// Pinned `nearai/nearai-red-sandbox` release archive (`v0.1.4`, aarch64-apple-darwin).
/// SHA-256 verified against the GitHub release asset digest (`5c1ae7b1...`).
pub const RTS_RELEASE: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.4/rts-0.1.4-aarch64-apple-darwin.tar.gz",
    sha256: [
        0x5c, 0x1a, 0xe7, 0xb1, 0xf9, 0xee, 0xbd, 0x59, 0xc8, 0x32, 0x9f, 0xc6, 0xa3, 0x20, 0x56,
        0x73, 0x5d, 0xb6, 0xbd, 0xe2, 0xcd, 0x76, 0x72, 0xc5, 0x92, 0x77, 0x51, 0x82, 0xb8, 0x53,
        0xc9, 0xb4,
    ],
};
