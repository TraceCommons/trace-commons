//! Process-scoped choices never modify the persisted local or signed layers.
use super::*;

pub(super) fn parse(
    raw: Option<&str>,
    registry: &Registry,
    profile: &Profile,
) -> Result<BTreeMap<String, bool>, &'static str> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    if raw.len() > MAX_MANIFEST_BYTES {
        return Err("feature-environment-overrides-too-large");
    }
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let flags = local::unique_flags(&mut deserializer)
        .map_err(|_| "feature-environment-overrides-invalid")?;
    deserializer
        .end()
        .map_err(|_| "feature-environment-overrides-invalid")?;
    if flags
        .keys()
        .any(|id| !local::allowed(registry, profile, id))
    {
        return Err("feature-environment-override-unavailable");
    }
    Ok(flags)
}
