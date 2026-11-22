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

    #[cfg(any(target_os = "macos", windows))]
    pub fn to_archive(self) -> rts::setup::ReleaseArchive {
        rts::setup::ReleaseArchive {
            url: self.url.to_owned(),
            sha256: self.sha256,
        }
    }
}

/// Pinned `nearai/nearai-red-sandbox` release archive (`v0.1.7`, aarch64-apple-darwin).
/// SHA-256 from the release's `SHA256SUMS` (`3de16682...`), checked against the
/// downloaded archive.
pub const RTS_RELEASE: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.7/rts-0.1.7-aarch64-apple-darwin.tar.gz",
    sha256: [
        0x3d, 0xe1, 0x66, 0x82, 0x7c, 0x4c, 0x40, 0x13, 0x5d, 0x09, 0x03, 0x36, 0x06, 0x64, 0xd9,
        0xc0, 0x9f, 0x30, 0x34, 0x3c, 0x47, 0x77, 0xbb, 0xc9, 0x1f, 0x37, 0x4b, 0xbc, 0x84, 0x78,
        0x3d, 0x6d,
    ],
};

/// Pinned release archive for Windows (`x86_64-pc-windows-msvc`: `rts.exe`,
/// `rts-egress.exe` and the plugin), from the same `nearai/nearai-red-sandbox`
/// `v0.1.7` release as the macOS one. SHA-256 from the release's `SHA256SUMS`
/// (`2b898e2c...`), checked against the downloaded archive.
pub const RTS_RELEASE_WINDOWS: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.7/rts-0.1.7-x86_64-pc-windows-msvc.tar",
    sha256: [
        0x2b, 0x89, 0x8e, 0x2c, 0xad, 0x41, 0xca, 0x1f, 0xbf, 0xf1, 0xd1, 0x06, 0x97, 0x35, 0x36,
        0x6c, 0x92, 0x47, 0x6b, 0x90, 0xb9, 0xde, 0xb8, 0x70, 0x48, 0xc8, 0x44, 0x30, 0xe6, 0xf3,
        0xd8, 0x23,
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
    fn both_pins_are_the_verified_0_1_7_digests() {
        let hex = |pin: &ReleasePin| {
            pin.sha256
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex(&RTS_RELEASE),
            "3de166827c4c40135d0903360664d9c09f30343c4777bbc91f374bbc84783d6d"
        );
        assert_eq!(
            hex(&RTS_RELEASE_WINDOWS),
            "2b898e2cad41ca1fbff1d1069735366c92476b90b9deb87048c84430e6f3d823"
        );
        assert!(
            RTS_RELEASE
                .url
                .ends_with("v0.1.7/rts-0.1.7-aarch64-apple-darwin.tar.gz")
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
        assert!(
            RTS_RELEASE_WINDOWS.url.starts_with(
                "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.7/"
            )
        );
        assert!(
            RTS_RELEASE_WINDOWS
                .url
                .ends_with("rts-0.1.7-x86_64-pc-windows-msvc.tar")
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
