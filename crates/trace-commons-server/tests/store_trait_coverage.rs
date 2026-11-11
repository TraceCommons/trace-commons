// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `PgBackend` must implement every method of `TraceCorpusStore` and
//! `Database` itself.
//!
//! Both traits give every method a default that returns a "backend missing"
//! error, so a test double overrides only the methods it uses. The cost is
//! that the compiler no longer notices a production backend missing a method:
//! deleting one from `PgBackend`, or adding a trait method without a
//! PostgreSQL body, still compiles, and the service then fails that call at
//! run time. This test restores the check by reading the sources.
//!
//! It relies on rustfmt layout, which CI enforces: an item opens and closes at
//! column 0, and its methods start at a four-space indent.

use std::collections::BTreeSet;
use std::path::Path;

fn source(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The names of the methods declared directly in the item that opens on the
/// first line starting with `header`.
fn item_methods(relative: &str, header: &str) -> BTreeSet<String> {
    let text = source(relative);
    let mut lines = text.lines().skip_while(|line| !line.starts_with(header));
    assert!(
        lines.next().is_some(),
        "{relative}: no line starts with `{header}`"
    );
    let mut methods = BTreeSet::new();
    for line in lines {
        if line == "}" {
            return methods;
        }
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        if let Some(rest) = rest.strip_prefix("fn ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            methods.insert(name);
        }
    }
    panic!("{relative}: `{header}` never closes at column 0");
}

fn assert_backend_covers(trait_file: &str, trait_header: &str, impl_file: &str, impl_header: &str) {
    let declared = item_methods(trait_file, trait_header);
    let implemented = item_methods(impl_file, impl_header);
    assert!(
        declared.len() > 50,
        "{trait_file}: read only {} methods from `{trait_header}`; has its layout changed?",
        declared.len()
    );
    let missing: Vec<_> = declared.difference(&implemented).collect();
    assert!(
        missing.is_empty(),
        "`{impl_header}` in {impl_file} relies on the error-returning trait default for {missing:?}; \
         implement each method for PostgreSQL"
    );
}

#[test]
fn pg_backend_implements_every_trace_corpus_store_method() {
    assert_backend_covers(
        "src/trace_corpus_storage.rs",
        "pub trait TraceCorpusStore",
        "src/db/trace_corpus_pg.rs",
        "impl TraceCorpusStore for PgBackend",
    );
}

#[test]
fn pg_backend_implements_every_database_method() {
    assert_backend_covers(
        "src/db/mod.rs",
        "pub trait Database",
        "src/db/postgres.rs",
        "impl Database for PgBackend",
    );
}
