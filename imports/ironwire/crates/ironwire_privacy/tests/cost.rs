//! What the filter costs on a large conversation.
//!
//! `docs/PRIVACY.md` §5 flags this: agents resend their whole history every
//! turn, so detection is O(history) per turn unless something is done about it.
//! Before adding a cache, measure — a cache that is not needed is complexity
//! with a correctness cost and no benefit.

use std::time::Instant;

use ironwire_privacy::{Detector, Exemptions, Salt, Tiers};

/// Roughly a 200k-token coding session: the point where a harness compacts.
fn large_history(turns: usize) -> serde_json::Value {
    let chunk = "pub fn reconcile(items: &[Item], budget: u64) -> Vec<Plan> {\n    \
                 items.iter().filter(|i| i.price <= budget).map(Plan::from).collect()\n}\n";
    serde_json::json!({
        "model": "claude-opus-4-6",
        "system": "You are Claude Code",
        "messages": (0..turns)
            .map(|i| serde_json::json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": format!("turn {i}\n{}", chunk.repeat(12)),
            }))
            .collect::<Vec<_>>(),
    })
}

/// Release-only. A debug build runs this ~25x slower (38 ms vs 1.5 ms on the
/// same history), so a budget asserted there measures the optimiser, not the
/// code — and would either fail on every `cargo test` or be loose enough to
/// mean nothing. CI runs it with `--release`.
#[test]
#[cfg_attr(debug_assertions, ignore = "perf budget is only meaningful in release")]
fn substituting_a_large_history_is_fast_enough_to_be_invisible() {
    // Measured at the *highest* deterministic level, so the budget covers what
    // a user who turns everything on actually pays. Three more pattern passes
    // move this: see the measured figure this test prints.
    let detector = Detector::new(&Tiers {
        secrets: true,
        named_values: vec!["acme-holdings.example-real.com".to_string()],
        pii: true,
    });
    let body = large_history(200);
    let bytes = body.to_string().len();

    // Warm, then measure.
    let _ = ironwire_privacy::substitute(&detector, &Salt::fixed(1), &Exemptions::default(), &body);
    let started = Instant::now();
    let runs = 5;
    for _ in 0..runs {
        let _ =
            ironwire_privacy::substitute(&detector, &Salt::fixed(1), &Exemptions::default(), &body);
    }
    let per_turn = started.elapsed() / runs;

    eprintln!(
        "history: {} KB, substitution: {:?} per turn",
        bytes / 1024,
        per_turn
    );

    // Measured: ~1.5ms at `credentials`, ~8ms at `pii` on this 333 KB history
    // — the three extra pattern passes cost roughly 5x, and that is the number
    // to watch if more classes are ever added.
    //
    // The budget is set against what it is competing with: a model's
    // time-to-first-token is hundreds of milliseconds at best. Anything under
    // ~50ms is invisible next to that, and buys us the right to skip a
    // detection cache entirely — which is a real simplification, since a cache
    // keyed on content is one more thing that can be wrong across a compaction
    // boundary.
    assert!(
        per_turn.as_millis() < 50,
        "substitution costs {per_turn:?} per turn on a {} KB history; that is \
         no longer invisible next to a model's first token, and the detection \
         cache in PRIVACY §5 needs building",
        bytes / 1024
    );
}
