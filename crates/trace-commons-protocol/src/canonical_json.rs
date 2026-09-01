//! Key-ordered JSON for the paths whose bytes are hashed.
//!
//! `serde_json`'s `preserve_order` feature swaps `serde_json::Map` from a
//! `BTreeMap` to an insertion-ordered `IndexMap`. Cargo unifies features
//! across a build, so one dependency anywhere in the workspace that turns it
//! on silently reorders every `Value::Object` in every crate -- and with it
//! every digest taken over untyped JSON. That is not hypothetical: adding
//! `dcap-qvl` on a branch enabled it and moved a golden envelope digest in a
//! crate the branch never touched.
//!
//! These helpers make that ordering explicit rather than inherited, so the
//! hashing paths emit the same bytes under either backing map. Under today's
//! feature set each one is a no-op -- a `BTreeMap` already iterates in key
//! order -- which is exactly why they can be adopted without moving a pinned
//! digest. [`canonicalize`] is the fix; the module's
//! `key_order_is_sorted_not_insertion_order` test is the alarm that fires if
//! `preserve_order` is ever switched on regardless.
//!
//! Only untyped JSON needs this. A `#[derive(Serialize)]` struct is written
//! field by field in declaration order by the serializer, with no map in the
//! way, so routing one through [`canonicalize`] would *change* its bytes
//! rather than pin them. Do not.

use serde_json::{Map, Value};

/// Rewrite every object in `value` so its keys are in sorted order,
/// recursing through nested objects and arrays.
///
/// A no-op today, by construction: rebuilding a `BTreeMap`-backed map from
/// its own sorted entries yields the identical map. Under `preserve_order`
/// it is what keeps the serialized bytes the same as they are today.
pub fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries = std::mem::take(map).into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut sorted = Map::new();
            for (key, mut entry) in entries {
                canonicalize(&mut entry);
                sorted.insert(key, entry);
            }
            *map = sorted;
        }
        Value::Array(items) => {
            for item in items {
                canonicalize(item);
            }
        }
        _ => {}
    }
}

/// [`canonicalize`] on a copy, for a value the caller only has by reference.
pub fn canonical_value(value: &Value) -> Value {
    let mut canonical = value.clone();
    canonicalize(&mut canonical);
    canonical
}

/// `serde_json::to_string` over key-ordered JSON.
pub fn to_canonical_string(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&canonical_value(value))
}

/// `serde_json::to_vec` over key-ordered JSON.
pub fn to_canonical_vec(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonical_value(value))
}

/// An object's keys in sorted order.
///
/// For the summarising paths that render key *names* into text: there the
/// iteration order survives into the output independently of any serializer,
/// and a truncated list makes it worse -- taking the first N of an
/// insertion-ordered map picks a different N, not merely a different order.
/// Sort before truncating.
pub fn sorted_keys(map: &Map<String, Value>) -> Vec<&str> {
    let mut keys = map.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

/// An object's entries in sorted key order. See [`sorted_keys`].
pub fn sorted_entries(map: &Map<String, Value>) -> Vec<(&str, &Value)> {
    let mut entries = map
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect::<Vec<_>>();
    entries.sort_unstable_by_key(|(key, _)| *key);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The alarm. If this fails, some dependency in the build has enabled
    /// `serde_json`'s `preserve_order` feature.
    ///
    /// Every hashing path that serializes a `serde_json::Value` depends on
    /// `Value::Object` iterating in key order, which is true only while
    /// `serde_json::Map` is backed by a `BTreeMap`. `preserve_order` swaps it
    /// for an insertion-ordered `IndexMap`, and because Cargo unifies
    /// features across a build, one dependency enabling it changes the bytes
    /// -- and therefore the digests -- produced by every crate here.
    #[test]
    fn key_order_is_sorted_not_insertion_order() {
        let mut object = Map::new();
        object.insert("zulu".to_string(), json!(1));
        object.insert("alpha".to_string(), json!(2));
        object.insert("mike".to_string(), json!(3));
        let serialized = serde_json::to_string(&Value::Object(object)).expect("serialize");

        assert_eq!(
            serialized, r#"{"alpha":2,"mike":3,"zulu":1}"#,
            "serde_json emitted object keys in insertion order rather than sorted order. \
             Some dependency in this build has enabled the serde_json `preserve_order` \
             feature, which swaps `serde_json::Map` from a BTreeMap to an insertion-ordered \
             IndexMap. Cargo unifies features across the whole build, so this affects every \
             crate here, including ones that never touch that dependency. Every digest taken \
             over untyped JSON -- envelope digests, redaction hashes, the NEAR outbox \
             idempotency key, drill evidence hashes -- changes silently as a result. Find the \
             new dependency with `cargo tree -e features -i serde_json` and drop it, or route \
             the affected paths through `canonical_json::canonicalize` before hashing."
        );
    }

    #[test]
    fn canonicalize_sorts_nested_objects_and_objects_in_arrays() {
        let mut value = json!({
            "zulu": {"delta": 1, "bravo": 2},
            "alpha": [{"yankee": 3, "xray": 4}],
        });
        canonicalize(&mut value);
        assert_eq!(
            serde_json::to_string(&value).expect("serialize"),
            r#"{"alpha":[{"xray":4,"yankee":3}],"zulu":{"bravo":2,"delta":1}}"#
        );
    }

    /// Canonical bytes do not depend on the order the value was built in --
    /// which is the same thing as saying they do not depend on which map
    /// backs `serde_json::Map`. Holds under either feature setting, unlike
    /// the guard above, which is meant to fail under `preserve_order`.
    #[test]
    fn canonical_bytes_ignore_the_order_the_value_was_built_in() {
        let unordered = json!({
            "zulu": {"delta": 1, "bravo": 2},
            "alpha": [{"yankee": 3, "xray": 4}, 5, null],
        });
        let built_in_sorted_order = json!({
            "alpha": [{"xray": 4, "yankee": 3}, 5, null],
            "zulu": {"bravo": 2, "delta": 1},
        });
        assert_eq!(
            to_canonical_string(&unordered).expect("serialize"),
            serde_json::to_string(&built_in_sorted_order).expect("serialize")
        );
        assert_eq!(
            to_canonical_vec(&unordered).expect("serialize"),
            to_canonical_string(&unordered)
                .expect("serialize")
                .into_bytes()
        );
    }

    #[test]
    fn sorted_keys_orders_before_truncation() {
        let mut object = Map::new();
        object.insert("zulu".to_string(), json!(1));
        object.insert("alpha".to_string(), json!(2));
        assert_eq!(sorted_keys(&object), vec!["alpha", "zulu"]);
        assert_eq!(
            sorted_entries(&object)
                .into_iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            vec!["alpha", "zulu"]
        );
    }
}
