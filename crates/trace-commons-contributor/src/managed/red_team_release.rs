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

/// Pinned release archive for Windows (`x86_64-pc-windows-msvc`: `rts.exe`,
/// `rts-egress.exe` and the plugin), from the same `nearai/nearai-red-sandbox`
/// release as the macOS one.
///
/// TODO(rts-0.1.7-windows-pin): the 0.1.7 release is not published yet, so
/// the digest is the all-zero placeholder. The test
/// `the_windows_pin_is_not_a_placeholder` fails until the real SHA-256 of
/// `rts-0.1.7-x86_64-pc-windows-msvc.tar` (from the release's `SHA256SUMS`)
/// replaces it, so a placeholder cannot ship.
pub const RTS_RELEASE_WINDOWS: ReleasePin = ReleasePin {
    url: "https://github.com/nearai/nearai-red-sandbox/releases/download/v0.1.7/rts-0.1.7-x86_64-pc-windows-msvc.tar",
    sha256: [0; 32],
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

    /// Red on purpose until the 0.1.7 release exists (see
    /// [`RTS_RELEASE_WINDOWS`]); it keeps a placeholder out of a release.
    #[test]
    fn the_windows_pin_is_not_a_placeholder() {
        assert!(
            !RTS_RELEASE_WINDOWS.is_placeholder(),
            "TODO(rts-0.1.7-windows-pin): replace the placeholder SHA-256 with the release's"
        );
    }
}
