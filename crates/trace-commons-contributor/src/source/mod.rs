//! Source model: the `TraceSource` trait, session/transcript types shared by
//! per-agent adapters, and deterministic hashing/id helpers.
//!
//! The adapters and types live in the `trace-commons-contributor-sources`
//! leaf crate, so daemon edits do not recompile them; everything public there
//! is re-exported here at its old path. What stays in this crate is what
//! needs the contributor itself: [`cli_source_roots`] resolves the staging
//! directory through [`ConfigStore`](crate::config::ConfigStore).

use std::path::Path;

pub use trace_commons_contributor_sources::source::*;

/// The CLI's sources: every native store at its conventional location, plus
/// trajectory files -- the path `--trajectory` named, or bounded
/// auto-discovery when it did not.
///
/// The staging directory is resolved through `ConfigStore`, so it honours
/// `TRACE_COMMONS_CONTRIBUTOR_DIR`, inherits the state directory's `0700`
/// mode, and is cleared by `logout` along with everything else there.
pub fn cli_source_roots(trajectory: Option<&Path>) -> SourceRoots {
    let selection = match trajectory {
        Some(path) => TrajectorySelection::Declared(path.to_path_buf()),
        None => TrajectorySelection::Auto {
            working_dir: std::env::current_dir().ok(),
            staging_dir: crate::config::ConfigStore::resolve(None)
                .ok()
                .map(|store| store.dir().join(TRAJECTORY_STAGING_SUBDIR)),
        },
    };
    SourceRoots::conventional().with_trajectory(selection)
}

#[cfg(test)]
mod cross_layer_tests;
