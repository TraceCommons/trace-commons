//! What the folder list and the maps build again, counted for the tests
//! that measure a redraw. Outside tests every call here does nothing.
//!
//! Kept apart from the code it counts: tests of `queue.rs` and
//! `monitor/map.rs` read their production source up to the first test
//! attribute, so a test-only line inside them would cut that read short.

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static ROWS_BUILT: Cell<usize> = const { Cell::new(0) };
    static REBUILD_ALL: Cell<bool> = const { Cell::new(false) };
    static LAYOUTS: Cell<usize> = const { Cell::new(0) };
}

/// A row of the folder list was built.
pub fn row_built() {
    #[cfg(test)]
    ROWS_BUILT.with(|built| built.set(built.get() + 1));
}

/// Whether the folder list builds every row again on every render, as it
/// did before it kept unchanged rows: only when a measuring test asks.
pub fn rebuild_all() -> bool {
    #[cfg(test)]
    return REBUILD_ALL.with(Cell::get);
    #[cfg(not(test))]
    false
}

/// A map laid its scene out again.
pub fn laid_out() {
    #[cfg(test)]
    LAYOUTS.with(|layouts| layouts.set(layouts.get() + 1));
}

/// Rows built so far. (Read by the mounted tests, which run on Linux.)
#[cfg(all(test, target_os = "linux"))]
pub fn rows_built() -> usize {
    ROWS_BUILT.with(Cell::get)
}

/// Map layouts so far.
#[cfg(all(test, target_os = "linux"))]
pub fn layouts() -> usize {
    LAYOUTS.with(Cell::get)
}

/// Makes the folder list build every row again on every render (`true`),
/// or keep unchanged ones.
#[cfg(all(test, target_os = "linux"))]
pub fn set_rebuild_all(on: bool) {
    REBUILD_ALL.with(|flag| flag.set(on));
}
