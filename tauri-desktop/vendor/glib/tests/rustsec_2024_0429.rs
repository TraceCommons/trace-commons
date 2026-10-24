// SPDX-License-Identifier: MIT
// Regression for https://rustsec.org/advisories/RUSTSEC-2024-0429.html.
// Run with --release: optimizations expose the immutable out-pointer bug.
use glib::prelude::*;

#[test]
fn string_iterator_preserves_forward_backward_and_mixed_iteration() {
    let strings = ["first", "second", "third", "fourth", "fifth"];
    let variant = strings.to_variant();

    assert_eq!(
        variant.array_iter_str().unwrap().collect::<Vec<_>>(),
        strings
    );
    assert_eq!(
        variant.array_iter_str().unwrap().rev().collect::<Vec<_>>(),
        strings.into_iter().rev().collect::<Vec<_>>()
    );

    let mut iter = variant.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some("first"));
    assert_eq!(iter.next_back(), Some("fifth"));
    assert_eq!(iter.nth(1), Some("third"));
    assert_eq!(iter.nth_back(0), Some("fourth"));
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);
    assert_eq!(variant.array_iter_str().unwrap().last(), Some("fifth"));
}
