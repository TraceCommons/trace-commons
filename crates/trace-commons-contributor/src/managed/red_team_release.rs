//! Pinned signed `rts` release archive installed into `<config>/red-team/runtime`.

/// A pinned, signed `rts` release archive (URL and 32-byte SHA-256 digest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleasePin {
    pub url: &'static str,
    pub sha256: [u8; 32],
}

impl ReleasePin {
    /// Whether this pin is the all-zero placeholder, i.e. not yet pinned.
    pub const fn is_placeholder(&self) -> bool {
        let mut i = 0;
        while i < 32 {
            if self.sha256[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    #[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
    pub fn to_archive(self) -> rts::setup::ReleaseArchive {
        rts::setup::ReleaseArchive {
            url: self.url.to_owned(),
            sha256: self.sha256,
        }
    }
}

/// Pinned `nearai/nearai-red-sandbox` release archive (`v0.1.10`, aarch64-apple-darwin).
/// SHA-256 from the release's `SHA256SUMS` (`9098b444...`), checked against the
/// downloaded archive.
pub const RTS_RELEASE: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.10/rts-0.1.10-aarch64-apple-darwin.tar.gz",
    sha256: [
        0x90, 0x98, 0xb4, 0x44, 0x5a, 0x43, 0x1c, 0x8c, 0x21, 0xcf, 0xf1, 0x3f, 0xef, 0x0c, 0x0d,
        0x70, 0x96, 0x33, 0x86, 0xdb, 0x67, 0x12, 0x1e, 0x97, 0x6a, 0x94, 0xe3, 0xa4, 0xbb, 0x15,
        0xdb, 0xdc,
    ],
};

/// Pinned release archive for Windows (`x86_64-pc-windows-msvc`: `rts.exe`,
/// `rts-egress.exe` and the plugin), from the same `nearai/nearai-red-sandbox`
/// `v0.1.10` release as the macOS one. SHA-256 from the release's `SHA256SUMS`
/// (`d76da1e6...`), checked against the downloaded archive.
pub const RTS_RELEASE_WINDOWS: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.10/rts-0.1.10-x86_64-pc-windows-msvc.tar",
    sha256: [
        0xd7, 0x6d, 0xa1, 0xe6, 0x8e, 0xe7, 0xfb, 0x98, 0xc4, 0x6a, 0xf7, 0xed, 0xe7, 0xaa, 0xe5,
        0x99, 0xf9, 0xd9, 0x16, 0x8a, 0x0f, 0x91, 0x9e, 0xca, 0xbd, 0x19, 0x32, 0x45, 0x2a, 0x34,
        0x63, 0x7d,
    ],
};

/// The pin for this platform's Red Team runtime.
#[cfg(windows)]
pub const fn platform_release() -> ReleasePin {
    RTS_RELEASE_WINDOWS
}

/// The pin for this platform's Red Team runtime.
#[cfg(not(windows))]
pub const fn platform_release() -> ReleasePin {
    RTS_RELEASE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_macos_pin_is_a_real_digest_of_a_github_release() {
        assert!(!RTS_RELEASE.is_placeholder());
        assert!(
            RTS_RELEASE
                .url
                .starts_with("https://github.com/nearai/nearai-red-sandbox/releases/download/")
        );
    }

    #[test]
    fn both_pins_are_the_verified_0_1_10_digests() {
        let hex = |pin: &ReleasePin| {
            pin.sha256
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex(&RTS_RELEASE),
            "9098b4445a431c8c21cff13fef0c0d70963386db67121e976a94e3a4bb15dbdc"
        );
        assert_eq!(
            hex(&RTS_RELEASE_WINDOWS),
            "d76da1e68ee7fb98c46af7ede7aae599f9d9168a0f919ecabd1932452a34637d"
        );
        assert!(
            RTS_RELEASE
                .url
                .ends_with("v0.1.10/rts-0.1.10-aarch64-apple-darwin.tar.gz")
        );
    }

    #[test]
    fn a_placeholder_is_recognised() {
        assert!(
            ReleasePin {
                url: "",
                sha256: [0; 32]
            }
            .is_placeholder()
        );
        let mut one = [0; 32];
        one[31] = 1;
        assert!(
            !ReleasePin {
                url: "",
                sha256: one
            }
            .is_placeholder()
        );
    }

    #[test]
    fn the_windows_pin_names_the_msvc_tar_of_the_same_release() {
        assert!(RTS_RELEASE_WINDOWS.url.starts_with(
            "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.10/"
        ));
        assert!(
            RTS_RELEASE_WINDOWS
                .url
                .ends_with("rts-0.1.10-x86_64-pc-windows-msvc.tar")
        );
    }

    /// Keeps a placeholder out of a release.
    #[test]
    fn the_windows_pin_is_not_a_placeholder() {
        assert!(
            !RTS_RELEASE_WINDOWS.is_placeholder(),
            "the Windows pin is still the all-zero placeholder"
        );
    }
}
