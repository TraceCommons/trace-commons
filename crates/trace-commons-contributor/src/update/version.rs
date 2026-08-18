#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VersionError {
    /// Not a three-part numeric version. `release-apps.yml` refuses to cut a
    /// tag that is not `X.Y.Z`, so anything else here is a malformed or
    /// hostile manifest rather than a version this build should reason about.
    #[error("update_version_malformed")]
    Malformed,
}

fn parse(v: &str) -> Result<[u64; 3], VersionError> {
    let mut parts = v.split('.');
    let mut out = [0u64; 3];
    for slot in out.iter_mut() {
        let raw = parts.next().ok_or(VersionError::Malformed)?;
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(VersionError::Malformed);
        }
        *slot = raw.parse().map_err(|_| VersionError::Malformed)?;
    }
    if parts.next().is_some() {
        return Err(VersionError::Malformed);
    }
    Ok(out)
}

/// True when `offered` is strictly greater than `current`.
///
/// Strictly: equal is false, so a replayed manifest for the running version
/// installs nothing, and older is false, so a replayed manifest for an older
/// version cannot walk a client backwards onto a build with known problems.
pub fn is_newer(current: &str, offered: &str) -> Result<bool, VersionError> {
    Ok(parse(offered)? > parse(current)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_patch_is_newer() {
        assert!(is_newer("0.1.0", "0.1.1").unwrap());
    }

    #[test]
    fn a_higher_minor_is_newer() {
        assert!(is_newer("0.1.9", "0.2.0").unwrap());
    }

    #[test]
    fn a_higher_major_is_newer() {
        assert!(is_newer("0.9.9", "1.0.0").unwrap());
    }

    #[test]
    fn components_compare_numerically_not_lexically() {
        // The bug this guards: "10" < "9" as strings.
        assert!(is_newer("0.9.0", "0.10.0").unwrap());
        assert!(!is_newer("0.10.0", "0.9.0").unwrap());
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("1.2.3", "1.2.3").unwrap());
    }

    #[test]
    fn an_older_version_is_not_newer() {
        assert!(!is_newer("1.2.3", "1.2.2").unwrap());
        assert!(!is_newer("2.0.0", "1.9.9").unwrap());
    }

    #[test]
    fn malformed_versions_are_refused_not_guessed() {
        assert!(is_newer("1.2", "1.2.3").is_err());
        assert!(is_newer("1.2.3", "1.2.3.4").is_err());
        assert!(is_newer("1.2.3", "v1.2.4").is_err());
        assert!(is_newer("1.2.3", "1.2.x").is_err());
        assert!(is_newer("", "1.2.3").is_err());
    }
}
