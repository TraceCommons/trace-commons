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

    #[cfg(red_team_platform)]
    pub fn to_archive(self) -> rts::setup::ReleaseArchive {
        rts::setup::ReleaseArchive {
            url: self.url.to_owned(),
            sha256: self.sha256,
        }
    }
}

/// Pinned `nearai/nearai-red-sandbox` release archive (`v0.1.8`, aarch64-apple-darwin).
/// SHA-256 from the release's `SHA256SUMS` (`c20de9da...`), checked against the
/// downloaded archive.
pub const RTS_RELEASE: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.8/rts-0.1.8-aarch64-apple-darwin.tar.gz",
    sha256: [
        0xc2, 0x0d, 0xe9, 0xda, 0x96, 0x1f, 0x09, 0xf1, 0xcb, 0x55, 0x57, 0xd6, 0xa1, 0x66, 0x6b,
        0x54, 0x66, 0x6f, 0xc9, 0xfe, 0x93, 0xad, 0x82, 0xe0, 0xf9, 0xd0, 0xc0, 0x0e, 0x24, 0x17,
        0x2f, 0x24,
    ],
};

/// Pinned release archive for Windows (`x86_64-pc-windows-msvc`: `rts.exe`,
/// `rts-egress.exe` and the plugin), from the same `nearai/nearai-red-sandbox`
/// `v0.1.8` release as the macOS one. SHA-256 from the release's `SHA256SUMS`
/// (`62004d3a...`), checked against the downloaded archive.
pub const RTS_RELEASE_WINDOWS: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.8/rts-0.1.8-x86_64-pc-windows-msvc.tar",
    sha256: [
        0x62, 0x00, 0x4d, 0x3a, 0xeb, 0xa4, 0x57, 0xbb, 0x42, 0x16, 0xb1, 0x53, 0x2e, 0x99, 0x50,
        0xed, 0xf3, 0xa1, 0x19, 0xc0, 0x0b, 0xa3, 0x09, 0x25, 0xde, 0x99, 0x6b, 0x20, 0x2e, 0x4a,
        0x5d, 0xc9,
    ],
};

/// Pinned release archive for x86-64 Linux (`rts-0.1.8-x86_64-unknown-linux-gnu.tar.gz`:
/// `bin/rts` and the plugin, no supervisor), from the same
/// `nearai/nearai-red-sandbox` release as the others.
///
/// SHA-256 from the release's `SHA256SUMS` (`711c5184...`), checked against the
/// downloaded archive.
pub const RTS_RELEASE_LINUX: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.8/rts-0.1.8-x86_64-unknown-linux-gnu.tar.gz",
    sha256: [
        0x71, 0x1c, 0x51, 0x84, 0x3e, 0x6a, 0xd4, 0x3d, 0x68, 0x93, 0xaa, 0x6e, 0xb5, 0x00, 0xb2,
        0xa8, 0x0c, 0xf1, 0x51, 0xf4, 0x4a, 0x61, 0x05, 0x08, 0x7f, 0xd6, 0x3e, 0xfa, 0x9d, 0xe6,
        0x2d, 0x76,
    ],
};

/// The pin for this platform's Red Team runtime.
#[cfg(windows)]
pub const fn platform_release() -> ReleasePin {
    RTS_RELEASE_WINDOWS
}

/// The pin for this platform's Red Team runtime.
#[cfg(target_os = "linux")]
pub const fn platform_release() -> ReleasePin {
    RTS_RELEASE_LINUX
}

/// The pin for this platform's Red Team runtime.
#[cfg(not(any(windows, target_os = "linux")))]
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
    fn all_three_pins_are_the_verified_0_1_8_digests() {
        let hex = |pin: &ReleasePin| {
            pin.sha256
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex(&RTS_RELEASE),
            "c20de9da961f09f1cb5557d6a1666b54666fc9fe93ad82e0f9d0c00e24172f24"
        );
        assert_eq!(
            hex(&RTS_RELEASE_LINUX),
            "711c51843e6ad43d6893aa6eb500b2a80cf151f44a6105087fd63efa9de62d76"
        );
        assert_eq!(
            hex(&RTS_RELEASE_WINDOWS),
            "62004d3aeba457bb4216b1532e9950edf3a119c00ba30925de996b202e4a5dc9"
        );
        assert!(
            RTS_RELEASE
                .url
                .ends_with("v0.1.8/rts-0.1.8-aarch64-apple-darwin.tar.gz")
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
                "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.8/"
            )
        );
        assert!(
            RTS_RELEASE_WINDOWS
                .url
                .ends_with("rts-0.1.8-x86_64-pc-windows-msvc.tar")
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

    #[test]
    fn the_linux_pin_names_the_x86_64_gnu_tarball_of_its_own_release() {
        assert!(
            RTS_RELEASE_LINUX.url.starts_with(
                "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.8/"
            )
        );
        assert!(
            RTS_RELEASE_LINUX
                .url
                .ends_with("rts-0.1.8-x86_64-unknown-linux-gnu.tar.gz")
        );
    }

    /// Keeps a placeholder out of a release.
    #[test]
    fn the_linux_pin_is_not_a_placeholder() {
        assert!(
            !RTS_RELEASE_LINUX.is_placeholder(),
            "the Linux pin is still the all-zero placeholder"
        );
    }
}
