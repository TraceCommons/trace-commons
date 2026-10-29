//! Several native profile roots behind one adapter routing name.
use super::{SessionRef, SessionTranscript, TraceSource};
use std::path::{Path, PathBuf};

pub(super) struct MultiRootSource {
    name: &'static str,
    sources: Vec<Box<dyn TraceSource>>,
}

impl MultiRootSource {
    pub(super) fn new(name: &'static str, sources: Vec<Box<dyn TraceSource>>) -> Self {
        Self { name, sources }
    }

    fn owner(&self, path: &Path) -> Option<&dyn TraceSource> {
        self.sources
            .iter()
            .find(|source| source.session_for_path(path).is_some())
            .map(|source| source.as_ref())
    }
}

impl TraceSource for MultiRootSource {
    fn name(&self) -> &'static str {
        self.name
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut found = Vec::new();
        for source in &self.sources {
            found.extend(source.discover()?);
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.dedup_by(|a, b| a.path == b.path);
        Ok(found)
    }

    fn discover_unopened(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut found = Vec::new();
        for source in &self.sources {
            found.extend(source.discover_unopened()?);
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.dedup_by(|a, b| a.path == b.path);
        Ok(found)
    }

    fn discover_insights_only(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut found = Vec::new();
        for source in &self.sources {
            found.extend(source.discover_insights_only()?);
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.dedup_by(|a, b| a.path == b.path);
        Ok(found)
    }

    fn load(&self, session: &SessionRef) -> anyhow::Result<SessionTranscript> {
        self.owner(&session.path)
            .ok_or_else(|| anyhow::anyhow!("native-session-outside-roots"))?
            .load(session)
    }

    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        self.sources
            .iter()
            .find_map(|source| source.session_for_path(path))
    }

    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        match self.owner(path) {
            Some(source) => source.session_at(path),
            None => Ok(None),
        }
    }
}
