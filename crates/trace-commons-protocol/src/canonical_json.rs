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
//! hashing paths emit the same bytes under either backing map. Adopting them
//! moved no pinned digest, because under a `BTreeMap` each one is a no-op.
//!
//! **`preserve_order` is now on in some of this workspace's build graphs and
//! off in others, and that is fine.** `dcap-qvl`'s mandatory `std` feature
//! depends on `serde_json/preserve_order`, so any graph containing
//! `dcap-qvl` -- `cargo test --workspace`, or anything reaching
//! `trace-commons-server` -- resolves `serde_json::Map` to an `IndexMap`.
//! A graph without it -- `cargo test -p trace-commons-protocol`, or a
//! permissive crate built standalone -- still gets a `BTreeMap`. Run
//! `cargo tree -e features -i serde_json` to see which one a given build
//! has.
//!
//! That is the condition this module was written to survive, so it is not an
//! alarm to be raised; the invariant is that **every path whose bytes are
//! hashed routes through [`canonicalize`], so the ordering of the backing map
//! cannot be observed in a digest.** The split has a useful consequence:
//! sorting a `serde_json::Map` is unobservable under a `BTreeMap`, which used
//! to make this module's map-level tests vacuous, and under `preserve_order`
//! they are real. The same tests now run both ways in different CI jobs.
//!
//! Only untyped JSON needs this. A `#[derive(Serialize)]` struct is written
//! field by field in declaration order by the serializer, with no map in the
//! way, so routing one through [`canonicalize`] would *change* its bytes
//! rather than pin them. Do not.

use serde_json::{Map, Value};

/// Rewrite every object in `value` so its keys are in sorted order,
/// recursing through nested objects and arrays.
///
/// A no-op under a `BTreeMap`, by construction: rebuilding one from its own
/// sorted entries yields the identical map. Under `preserve_order` it does
/// real work, and is what keeps the serialized bytes the same as they are
/// under a `BTreeMap`.
pub fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries = std::mem::take(map).into_iter().collect::<Vec<_>>();
            sort_entries(&mut entries);
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

/// The ordering decision, on input a caller can hand over out of order.
///
/// Every public entry point in this module routes its comparison through
/// here, and it exists as a separate function so that comparison is
/// *testable* even in a build whose `serde_json::Map` is a `BTreeMap`. There,
/// sorting a map is unobservable -- anything a test puts in one is already
/// sorted, and sorting it again proves nothing -- but a `Vec` the test builds
/// is not, so `sort_entries_orders_input_the_caller_built` below fails if
/// this comparison is dropped or reversed.
///
/// What a `Vec`-level test cannot catch is `canonicalize` failing to *call*
/// this at all, since an early return is likewise unobservable under a
/// `BTreeMap`. In a graph that pulls `dcap-qvl` the map is an `IndexMap` and
/// `canonicalize_yields_sorted_bytes_whatever_map_backs_this_build` catches
/// it directly; the `serde_json preserve_order guard` CI job covers the
/// graphs that do not, by forcing the feature on where nothing enables it.
fn sort_entries<K: Ord, V>(entries: &mut [(K, V)]) {
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
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
    sorted_entries(map)
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// An object's entries in sorted key order. See [`sorted_keys`].
pub fn sorted_entries(map: &Map<String, Value>) -> Vec<(&str, &Value)> {
    let mut entries = map
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect::<Vec<_>>();
    sort_entries(&mut entries);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The alarm, retargeted.
    ///
    /// It used to assert that `serde_json` emits sorted keys on its own,
    /// i.e. that nothing in the build had enabled `preserve_order`. That
    /// premise was right while nothing enabled it and is wrong now that
    /// `dcap-qvl` does. The invariant worth guarding was never "the feature
    /// is off" -- it is **"the hashed bytes do not depend on which map backs
    /// `serde_json::Map`"**, which is what this module exists to provide.
    ///
    /// So this now asserts that [`canonicalize`] sorts an object built out
    /// of order, whatever this build resolved `serde_json::Map` to. The
    /// second half asserts the test is not vacuous: under an `IndexMap` the
    /// uncanonicalized bytes really are unsorted, so `canonicalize` is
    /// observed doing work rather than merely agreeing with a `BTreeMap`.
    #[test]
    fn canonicalize_yields_sorted_bytes_whatever_map_backs_this_build() {
        let mut object = Map::new();
        object.insert("zulu".to_string(), json!(1));
        object.insert("alpha".to_string(), json!(2));
        object.insert("mike".to_string(), json!(3));
        let value = Value::Object(object);

        let raw = serde_json::to_string(&value).expect("serialize");
        let canonical = to_canonical_string(&value).expect("serialize");

        assert_eq!(
            canonical, r#"{"alpha":2,"mike":3,"zulu":1}"#,
            "canonicalize did not emit object keys in sorted order. Every digest taken over \
             untyped JSON -- envelope digests, redaction hashes, the NEAR outbox idempotency \
             key, drill evidence hashes -- is stable only because this function sorts. \
             `serde_json::Map` is a BTreeMap by default and an insertion-ordered IndexMap \
             under the `preserve_order` feature, which `dcap-qvl` enables through its \
             mandatory `std` feature; Cargo unifies features across a build, so which map \
             backs this one depends on the graph it resolved (`cargo tree -e features -i \
             serde_json`). Any hashing path that serializes a Value without routing it \
             through here emits different bytes in the two cases. Fix the path, not this test."
        );

        // Which map this build got, asserted rather than assumed -- and a
        // real assertion either way, so this cannot quietly become a
        // tautology in whichever graph it happens to run under.
        if raw == canonical {
            // BTreeMap. Sorting is unobservable through the map here, so the
            // assertion above is satisfied whether or not `canonicalize` did
            // anything; `sort_entries_orders_input_the_caller_built` and the
            // `serde_json preserve_order guard` CI job are what carry it.
            assert_eq!(raw, r#"{"alpha":2,"mike":3,"zulu":1}"#);
        } else {
            // IndexMap. `canonicalize` is observed turning insertion order
            // into sorted order, which is the whole claim.
            assert_eq!(
                raw, r#"{"zulu":1,"alpha":2,"mike":3}"#,
                "serde_json::Map is insertion-ordered in this build, but the uncanonicalized \
                 bytes are neither insertion order nor sorted order. Something other than the \
                 backing map changed."
            );
        }
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
    /// backs `serde_json::Map`. Real under an `IndexMap`, where the two
    /// `json!` literals below genuinely differ before canonicalization.
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

    /// The comparison itself, on input this test built out of order.
    ///
    /// This is the one assertion in the module that can fail if the sort is
    /// deleted **in every build graph**. The map-level tests route through a
    /// `serde_json::Map`, which in a graph without `dcap-qvl` is a `BTreeMap`
    /// and hands back sorted keys whether or not this module does any work.
    #[test]
    fn sort_entries_orders_input_the_caller_built() {
        let mut entries = vec![("zulu", 1), ("alpha", 2), ("mike", 3)];
        sort_entries(&mut entries);
        assert_eq!(entries, vec![("alpha", 2), ("mike", 3), ("zulu", 1)]);

        // The owned-key shape `canonicalize` uses, and a duplicate-free
        // ordering over more than three elements so a partial sort shows up.
        let mut owned = vec![
            ("zulu".to_string(), json!(1)),
            ("alpha".to_string(), json!(2)),
            ("yankee".to_string(), json!(3)),
            ("bravo".to_string(), json!(4)),
            ("mike".to_string(), json!(5)),
        ];
        sort_entries(&mut owned);
        assert_eq!(
            owned
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "bravo", "mike", "yankee", "zulu"]
        );
    }

    /// The nested case of
    /// `canonicalize_yields_sorted_bytes_whatever_map_backs_this_build`:
    /// objects inside objects and inside arrays, which that one does not
    /// cover. Same caveat -- real under an `IndexMap`, vacuous under a
    /// `BTreeMap`.
    #[test]
    fn canonicalize_emits_sorted_bytes_for_an_out_of_order_object() {
        let mut object = Map::new();
        object.insert("zulu".to_string(), json!({"delta": 1, "bravo": 2}));
        object.insert("alpha".to_string(), json!([{"yankee": 3, "xray": 4}]));
        let mut value = Value::Object(object);
        canonicalize(&mut value);
        assert_eq!(
            serde_json::to_string(&value).expect("serialize"),
            r#"{"alpha":[{"xray":4,"yankee":3}],"zulu":{"bravo":2,"delta":1}}"#
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
