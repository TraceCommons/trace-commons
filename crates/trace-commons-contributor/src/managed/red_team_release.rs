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

/// Pinned `nearai/nearai-red-sandbox` release archive (`v0.1.6`, aarch64-apple-darwin).
/// SHA-256 verified against the GitHub release asset digest (`a49a7a6a...`).
pub const RTS_RELEASE: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.6/rts-0.1.6-aarch64-apple-darwin.tar.gz",
    sha256: [
        0xa4, 0x9a, 0x7a, 0x6a, 0xf9, 0xea, 0xcb, 0xdc, 0x2f, 0xcb, 0xfa, 0x9f, 0x8a, 0x41, 0x80,
        0xee, 0xc6, 0x30, 0x43, 0x8d, 0xfe, 0xd2, 0x44, 0x6b, 0xa6, 0xd3, 0x94, 0x0b, 0x78, 0x9e,
        0xba, 0xa1,
    ],
};
