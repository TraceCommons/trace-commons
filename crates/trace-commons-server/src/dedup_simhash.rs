// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic 64-bit token simhash over a trace's canonical text, for
//! cross-trace duplicate clustering. Word-shingle features, FNV-1a hashed for
//! build-stable reproducibility. Pure: no I/O.
//!
//! Two algorithms live here, named by [`DedupAlgorithm`]: the multiset
//! 2-shingle v1 the corpus was first derived under, and the set-semantic
//! 3-shingle v2 that replaces it. Both are kept because a stored row names
//! the algorithm that produced it and the re-derivation pass can target
//! either. Which one the inline gate path uses is [`ACTIVE_DEDUP_ALGORITHM`],
//! v2 since the re-derivation pass moved the corpus onto it; see
//! `dedup_assign.rs` for the rollout rules that govern moving it.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The simhash algorithms this build can derive, as a closed set. Each
/// variant names one function and one set of clustering constants
/// ([`crate::dedup_assign::DedupAlgorithm::constants`]); the name is the
/// second half of a `dedup_signal_version` stamp and is what the
/// re-derivation route parses its `algorithm` parameter into. Serialized as
/// the name, so an unknown name is a deserialization error rather than a
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum DedupAlgorithm {
    /// Multiset simhash over overlapping 2-token shingles: every occurrence
    /// of a shingle votes. On scaffolding-heavy long traces the repeated
    /// shingles own the vote and unrelated sessions from one harness
    /// collide; kept so stored rows can be read, targeted and rolled back.
    V1,
    /// Set simhash over overlapping 3-token shingles: each distinct shingle
    /// votes once, however often it occurs. Width falls back to 2 for a
    /// two-token text and to the unigram for a one-token text.
    V2,
}

impl DedupAlgorithm {
    /// The algorithm half of the `<render>+<simhash>` stamp.
    pub const fn name(self) -> &'static str {
        match self {
            DedupAlgorithm::V1 => "fnv1a-2shingle.v1",
            DedupAlgorithm::V2 => "fnv1a-3shingle-set.v2",
        }
    }

    /// The function this name stands for.
    pub fn simhash(self, canonical_text: &str) -> u64 {
        match self {
            DedupAlgorithm::V1 => trace_simhash_v1(canonical_text),
            DedupAlgorithm::V2 => trace_simhash_v2(canonical_text),
        }
    }
}

impl fmt::Display for DedupAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for DedupAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [DedupAlgorithm::V1, DedupAlgorithm::V2]
            .into_iter()
            .find(|a| a.name() == s)
            .ok_or_else(|| "unknown dedup simhash algorithm".to_string())
    }
}

impl TryFrom<String> for DedupAlgorithm {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<DedupAlgorithm> for String {
    fn from(a: DedupAlgorithm) -> String {
        a.name().to_string()
    }
}

/// The algorithm the INLINE gate path derives and stamps with, and whose
/// constants it clusters under. This is the constant the rollout in
/// `dedup_assign.rs` is about: it moves only after the re-derivation pass
/// has completed on every production database, and never in the same
/// binary that introduced the pass. Moved to v2 on 2026-09-21; the pass that
/// preceded it is `POST /v1/admin/rederive-dedup`
/// (`docs/operator/dedup-recluster.md`).
pub const ACTIVE_DEDUP_ALGORITHM: DedupAlgorithm = DedupAlgorithm::V2;

/// Identifier for the simhash ALGORITHM the inline path uses: the name of
/// [`ACTIVE_DEDUP_ALGORITHM`]. Composed with the enclave's canonical render
/// version into `trace_gate_decisions.dedup_signal_version`, so a stored
/// simhash can say which algorithm produced it rather than only which number
/// it is — deterministic gate services stamp the same column with a digest
/// window and must never be clustered against a real simhash. Any change to
/// a tokenizer, a shingle width, or a hash is a new [`DedupAlgorithm`]
/// variant, not an edit to an existing one.
pub const DEDUP_SIMHASH_ALGORITHM: &str = ACTIVE_DEDUP_ALGORITHM.name();

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Lowercase, split on non-alphanumeric, drop empties. Deterministic and
/// dependency-free — a simhash does not need a linguistic tokenizer.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// The simhash the inline gate path derives: [`ACTIVE_DEDUP_ALGORITHM`]'s
/// function. Callers that must stay on one algorithm whatever the active one
/// is -- the correction path, which stores no stamp (#538) -- name
/// [`trace_simhash_v1`] directly instead.
pub fn trace_simhash(canonical_text: &str) -> u64 {
    ACTIVE_DEDUP_ALGORITHM.simhash(canonical_text)
}

/// `fnv1a-2shingle.v1`: multiset simhash over overlapping 2-token shingles
/// (unigram for a single token). Every occurrence of a shingle votes, so on a
/// long trace a shingle repeated a thousand times outweighs a thousand
/// shingles seen once -- see [`DedupAlgorithm::V1`].
pub fn trace_simhash_v1(canonical_text: &str) -> u64 {
    let toks = tokens(canonical_text);
    if toks.is_empty() {
        return 0;
    }
    let width = toks.len().min(2);
    vote(
        toks.windows(width)
            .map(|w| fnv1a_64(w.join(" ").as_bytes())),
    )
}

/// `fnv1a-3shingle-set.v2`: set simhash over overlapping 3-token shingles
/// (2-shingle for a two-token text, unigram for one token). Shingle hashes
/// are deduplicated before voting, so each distinct shingle contributes
/// exactly once however often it occurs -- see [`DedupAlgorithm::V2`].
pub fn trace_simhash_v2(canonical_text: &str) -> u64 {
    let toks = tokens(canonical_text);
    if toks.is_empty() {
        return 0;
    }
    let width = toks.len().min(3);
    let distinct: HashSet<u64> = toks
        .windows(width)
        .map(|w| fnv1a_64(w.join(" ").as_bytes()))
        .collect();
    vote(distinct.into_iter())
}

/// Majority vote of feature hashes into a 64-bit signature: each feature
/// adds +1 to the accumulator of every bit it has set and -1 to every bit it
/// has clear; a positive accumulator becomes a set bit. Shared by both
/// algorithms; they differ only in which features they feed it.
fn vote(features: impl Iterator<Item = u64>) -> u64 {
    let mut acc = [0i32; 64];
    for f in features {
        for (bit, slot) in acc.iter_mut().enumerate() {
            if (f >> bit) & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }
    let mut sig: u64 = 0;
    for (bit, slot) in acc.iter().enumerate() {
        if *slot > 0 {
            sig |= 1u64 << bit;
        }
    }
    sig
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        assert_eq!(
            trace_simhash("the quick brown fox jumps"),
            trace_simhash("the quick brown fox jumps")
        );
    }

    #[test]
    fn identical_text_zero_distance() {
        let t = "fn main() { let x = compute(); println!(\"{}\", x); }";
        assert_eq!(hamming_distance(trace_simhash(t), trace_simhash(t)), 0);
    }

    #[test]
    fn near_identical_small_distance() {
        // one token changed out of many -> small Hamming distance
        let a = "the agent debugged the parser and fixed the off by one error in the loop";
        let b = "the agent debugged the parser and fixed the off by one error in the block";
        assert!(
            hamming_distance(trace_simhash(a), trace_simhash(b)) <= 8,
            "near-identical texts should be close: {}",
            hamming_distance(trace_simhash(a), trace_simhash(b))
        );
    }

    #[test]
    fn distinctive_token_shim_still_close() {
        // A6: same content + a few injected nonce tokens -> still close (bulk tokens unchanged)
        let base = "the agent read the config file parsed the yaml and validated every required key in order";
        let shimmed = format!("{base} zqxnonce7731 vvblorpmarker9920");
        assert!(
            hamming_distance(trace_simhash(base), trace_simhash(&shimmed)) <= 10,
            "shim should stay close: {}",
            hamming_distance(trace_simhash(base), trace_simhash(&shimmed))
        );
    }

    #[test]
    fn unrelated_large_distance() {
        let a = "the agent debugged the parser and fixed the off by one error in the loop";
        let b =
            "quarterly revenue projections exceeded forecasts across every regional market segment";
        assert!(
            hamming_distance(trace_simhash(a), trace_simhash(b)) >= 18,
            "unrelated texts should be far: {}",
            hamming_distance(trace_simhash(a), trace_simhash(b))
        );
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(trace_simhash(""), 0);
        assert_eq!(trace_simhash_v2(""), 0);
    }

    /// The inline entry point is the function the active algorithm names,
    /// and since the flip that is v2. The legacy literal is frozen and must
    /// now differ from what the build stamps: that difference is the whole
    /// point of the stamp, and it is why the re-derivation pass had to
    /// complete before this build was installed.
    #[test]
    fn the_inline_simhash_is_the_active_algorithm() {
        assert_eq!(ACTIVE_DEDUP_ALGORITHM, DedupAlgorithm::V2);
        assert_eq!(DEDUP_SIMHASH_ALGORITHM, "fnv1a-3shingle-set.v2");
        let text = "the agent read the config file parsed the yaml and validated every key";
        assert_eq!(trace_simhash(text), trace_simhash_v2(text));
        assert_eq!(trace_simhash(text), ACTIVE_DEDUP_ALGORITHM.simhash(text));
        assert_ne!(
            format!("events.v1+{DEDUP_SIMHASH_ALGORITHM}"),
            crate::dedup_assign::LEGACY_DEDUP_SIGNAL_VERSION,
            "the build's stamp has moved off the frozen legacy literal"
        );
        assert_eq!(
            crate::dedup_assign::LEGACY_DEDUP_SIGNAL_VERSION,
            "events.v1+fnv1a-2shingle.v1",
            "and the legacy literal itself did not move"
        );
        // v1 stays reachable by name for stored rows and for a rollback pass.
        assert_eq!(DedupAlgorithm::V1.name(), "fnv1a-2shingle.v1");
        assert_ne!(trace_simhash_v1(text), 0);
    }

    /// `trace_simhash_v1` is the pre-refactor body moved, not rewritten:
    /// every stored v1 row depends on it producing the same number. The
    /// original body is kept here verbatim as the oracle.
    #[test]
    fn v1_is_byte_identical_to_the_original_implementation() {
        fn original(canonical_text: &str) -> u64 {
            let toks = tokens(canonical_text);
            if toks.is_empty() {
                return 0;
            }
            let mut features: Vec<u64> = Vec::new();
            if toks.len() == 1 {
                features.push(fnv1a_64(toks[0].as_bytes()));
            } else {
                for w in toks.windows(2) {
                    features.push(fnv1a_64(format!("{} {}", w[0], w[1]).as_bytes()));
                }
            }
            let mut acc = [0i32; 64];
            for f in features {
                for (bit, slot) in acc.iter_mut().enumerate() {
                    if (f >> bit) & 1 == 1 {
                        *slot += 1;
                    } else {
                        *slot -= 1;
                    }
                }
            }
            let mut sig: u64 = 0;
            for (bit, slot) in acc.iter().enumerate() {
                if *slot > 0 {
                    sig |= 1u64 << bit;
                }
            }
            sig
        }
        for text in [
            "",
            "alpha",
            "alpha beta",
            "the agent debugged the parser and fixed the off by one error in the loop",
            &render(&synthetic_trace(LAYOUT, 11)),
        ] {
            assert_eq!(trace_simhash_v1(text), original(text));
        }
    }

    #[test]
    fn algorithm_names_round_trip() {
        assert_eq!(DedupAlgorithm::V1.name(), "fnv1a-2shingle.v1");
        assert_eq!(DedupAlgorithm::V2.name(), "fnv1a-3shingle-set.v2");
        for algorithm in [DedupAlgorithm::V1, DedupAlgorithm::V2] {
            assert_eq!(algorithm.name().parse::<DedupAlgorithm>(), Ok(algorithm));
            let json = serde_json::to_value(algorithm).expect("serializes");
            assert_eq!(json, serde_json::json!(algorithm.name()));
            let back: DedupAlgorithm = serde_json::from_value(json).expect("deserializes");
            assert_eq!(back, algorithm);
        }
        assert!("fnv1a-2shingle.v2".parse::<DedupAlgorithm>().is_err());
        assert!(
            serde_json::from_value::<DedupAlgorithm>(serde_json::json!("digest-prefix.v1"))
                .is_err(),
            "a stamp that is not a simhash algorithm is not a target"
        );
        assert_eq!(
            DedupAlgorithm::V2.simhash("a b c d"),
            trace_simhash_v2("a b c d")
        );
    }

    #[test]
    fn v2_short_texts_fall_back_to_narrower_shingles() {
        // A one-token text hashes the unigram; two tokens hash one 2-shingle.
        // Neither is zero, and they differ from each other.
        let one = trace_simhash_v2("alpha");
        let two = trace_simhash_v2("alpha beta");
        assert_ne!(one, 0);
        assert_ne!(two, 0);
        assert_ne!(one, two);
        assert_eq!(one, trace_simhash_v1("alpha"), "unigram fallback is shared");
    }

    /// Set semantics: how OFTEN a shingle occurs does not move v2. A line
    /// repeated twice and the same line repeated a thousand times have the
    /// same set of distinct shingles (the line's own and the line-to-line
    /// boundary ones), so v2 is identical, while v1's multiset vote is owned
    /// by the repeated line.
    #[test]
    fn v2_ignores_occurrence_counts() {
        let head = "the agent debugged the parser and fixed the off by one error in the loop\n\
                    quarterly revenue projections exceeded forecasts across every regional market";
        let padded = |n: usize| {
            format!(
                "{head}\n{}",
                std::iter::repeat_n("tool_result (Bash): running 3 tests", n)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        assert_eq!(
            trace_simhash_v2(&padded(2)),
            trace_simhash_v2(&padded(1000))
        );
        assert_ne!(
            trace_simhash_v1(&padded(2)),
            trace_simhash_v1(&padded(1000)),
            "v1 is the multiset hash; the repeated line owns its vote"
        );
    }

    // ---- long synthetic traces with heavy scaffolding repetition ----

    /// Deterministic generator for long `events.v1`-shaped traces. No
    /// dependency: a 64-bit LCG seeded by the test.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// The fixed scaffolding a real harness repeats: event headers, JSON key
    /// runs, path prefixes, a test-runner banner. About fifty literal lines,
    /// so however often they recur they contribute a few hundred distinct
    /// shingles.
    const SCAFFOLDING: [&str; 50] = [
        "tool_result (Read): 1 use std::collections::HashMap;",
        "tool_result (Read): 2 use std::sync::Arc;",
        "tool_result (Read): 3 use anyhow::Result;",
        "tool_result (Read): 4 use serde::Deserialize;",
        "tool_result (Read): 5 use serde::Serialize;",
        "tool_result (Read): 6 use tokio::sync::RwLock;",
        "tool_result (Read): 7 use uuid::Uuid;",
        "tool_result (Read): 8 use chrono::DateTime;",
        "tool_result (Bash): Compiling trace-commons-server v0.1.0",
        "tool_result (Bash): Finished test profile unoptimized debuginfo target",
        "tool_result (Bash): Running unittests src/lib.rs",
        "tool_result (Bash): running 12 tests",
        "tool_result (Bash): test result: ok. 12 passed; 0 failed; 0 ignored",
        "tool_result (Bash): warning: unused variable",
        "tool_result (Bash): error[E0308]: mismatched types",
        "tool_result (Bash): expected struct String found reference &str",
        "tool_result (Grep): crates/trace-commons-server/src/bin/trace-commons-ingest.rs",
        "tool_result (Grep): crates/trace-commons-server/src/trace_corpus_storage.rs",
        "tool_result (Grep): crates/trace-commons-server/src/db/postgres.rs",
        "tool_result (Grep): crates/trace-commons-protocol/src/lib.rs",
        "tool_result (Read): { \"schema_version\": \"trace-contribution.v1\",",
        "tool_result (Read): \"submission_id\": \"00000000-0000-0000-0000-000000000000\",",
        "tool_result (Read): \"tenant_id\": \"tenant\", \"events\": [",
        "tool_result (Read): { \"event_type\": \"tool_result\", \"tool_name\": \"Bash\",",
        "tool_result (Read): \"redacted_content\": \"\" } ] }",
        "tool_result (Read): #[derive(Debug, Clone, PartialEq, Eq)]",
        "tool_result (Read): #[derive(Debug, Serialize, Deserialize)]",
        "tool_result (Read): #[tokio::test]",
        "tool_result (Read): #[test]",
        "tool_result (Read): fn main() {",
        "tool_result (Read): async fn handler(State(state): State<Arc<AppState>>) {",
        "tool_result (Read): let db = state.db_mirror.as_ref();",
        "tool_result (Read): .await",
        "tool_result (Read): .expect(\"query succeeds\");",
        "tool_result (Read): Ok(())",
        "tool_result (Read): }",
        "tool_result (Bash): On branch main nothing to commit working tree clean",
        "tool_result (Bash): diff --git a/src/lib.rs b/src/lib.rs",
        "tool_result (Bash): index 0000000..1111111 100644",
        "tool_result (Bash): @@ -1,4 +1,4 @@",
        "tool_result (Bash): $ cargo fmt --all -- --check",
        "tool_result (Bash): $ cargo clippy -p trace-commons-server --all-targets",
        "tool_result (Bash): Checking trace-commons-protocol v0.1.0",
        "tool_result (Bash): Checking trace-commons-gate-api v0.1.0",
        "tool_result (Bash): Checking trace-commons-gate-enclave v0.1.0",
        "tool_result (Bash): Only you see this output",
        "assistant_message: Let me look at the file.",
        "assistant_message: Now the tests.",
        "assistant_message: I will run the suite.",
        "user_message: continue",
    ];

    /// About three scaffold lines per content line, so scaffolding is
    /// roughly 70% of tokens.
    const SCAFFOLD_SHARE_PERCENT: u64 = 74;
    const CONTENT_VOCABULARY: u64 = 5_000;
    const CONTENT_LINE_TOKENS: usize = 12;
    const EVENTS_PER_TRACE: usize = 2_000;

    fn content_line(rng: &mut Lcg) -> String {
        let words: Vec<String> = (0..CONTENT_LINE_TOKENS)
            .map(|_| format!("w{}", rng.below(CONTENT_VOCABULARY)))
            .collect();
        format!("assistant_message: {}", words.join(" "))
    }

    /// One rendered trace: `EVENTS_PER_TRACE` lines, each a scaffold line or
    /// a content line, joined by newlines as `dedup_canonical_text` joins
    /// them. `layout_seed` fixes WHICH positions are scaffolding and which
    /// scaffold line each is; `content_seed` fixes the content tokens. Two
    /// traces with the same layout seed share their scaffolding exactly, as
    /// two sessions from one harness do.
    fn synthetic_trace(layout_seed: u64, content_seed: u64) -> Vec<String> {
        let mut layout = Lcg(layout_seed);
        let mut content = Lcg(content_seed);
        (0..EVENTS_PER_TRACE)
            .map(|_| {
                if layout.below(100) < SCAFFOLD_SHARE_PERCENT {
                    SCAFFOLDING[layout.below(SCAFFOLDING.len() as u64) as usize].to_string()
                } else {
                    content_line(&mut content)
                }
            })
            .collect()
    }

    fn render(lines: &[String]) -> String {
        lines.join("\n")
    }

    fn content_positions(lines: &[String]) -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with("assistant_message: w"))
            .map(|(i, _)| i)
            .collect()
    }

    /// Replace `percent` of the CONTENT tokens (scaffolding untouched) with
    /// fresh vocabulary drawn from `seed`.
    fn reword(lines: &[String], percent: u64, seed: u64) -> Vec<String> {
        let mut rng = Lcg(seed);
        lines
            .iter()
            .map(|line| {
                let Some(body) = line.strip_prefix("assistant_message: w") else {
                    return line.clone();
                };
                let words: Vec<String> = format!("w{body}")
                    .split(' ')
                    .map(|w| {
                        if rng.below(100) < percent {
                            format!("r{}", rng.below(CONTENT_VOCABULARY))
                        } else {
                            w.to_string()
                        }
                    })
                    .collect();
                format!("assistant_message: {}", words.join(" "))
            })
            .collect()
    }

    /// Replace every content line from `from_fraction_num/den` of the trace
    /// onwards: a session that forked there.
    fn fork_from(lines: &[String], num: usize, den: usize, seed: u64) -> Vec<String> {
        let mut rng = Lcg(seed);
        let cut = lines.len() * num / den;
        lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i >= cut && line.starts_with("assistant_message: w") {
                    content_line(&mut rng)
                } else {
                    line.clone()
                }
            })
            .collect()
    }

    /// Same project, different session: the same scaffolding, the same 40%
    /// block of "file read" content, and different conversation content.
    fn same_project_pair(layout_seed: u64, shared_seed: u64, a: u64, b: u64) -> (String, String) {
        let base = synthetic_trace(layout_seed, shared_seed);
        let positions = content_positions(&base);
        let shared_until = positions.len() * 40 / 100;
        let mut own_a = Lcg(a);
        let mut own_b = Lcg(b);
        let mut ta = base.clone();
        let mut tb = base.clone();
        for &i in &positions[shared_until..] {
            ta[i] = content_line(&mut own_a);
            tb[i] = content_line(&mut own_b);
        }
        (render(&ta), render(&tb))
    }

    const TAU_V2: u32 = crate::dedup_assign::DEDUP_CONSTANTS_V2.tau_hamming;
    const LAYOUT: u64 = 0x5eed_1a70;

    fn d2(a: &str, b: &str) -> u32 {
        hamming_distance(trace_simhash_v2(a), trace_simhash_v2(b))
    }

    #[test]
    fn v2_long_trace_identical_and_resubmitted_are_zero() {
        let a = render(&synthetic_trace(LAYOUT, 11));
        // A resubmission differs in submission id and timestamps, which the
        // render omits; the canonical text is byte-identical.
        let resubmitted = a.clone();
        assert_eq!(d2(&a, &a), 0);
        assert_eq!(d2(&a, &resubmitted), 0);
    }

    #[test]
    fn v2_long_trace_a6_shim_stays_within_two_bits() {
        let a = render(&synthetic_trace(LAYOUT, 11));
        let shimmed = format!("{a}\nassistant_message: zqxnonce7731 vvblorpmarker9920 qq81");
        assert!(
            d2(&a, &shimmed) <= 2,
            "shim moved v2 by {}",
            d2(&a, &shimmed)
        );
    }

    #[test]
    fn v2_long_trace_light_reword_stays_within_tau() {
        let base = synthetic_trace(LAYOUT, 11);
        let reworded = reword(&base, 2, 99);
        let d = d2(&render(&base), &render(&reworded));
        assert!(d <= TAU_V2, "2% content reword moved v2 by {d} > {TAU_V2}");
    }

    #[test]
    fn v2_long_trace_late_fork_stays_within_tau() {
        let base = synthetic_trace(LAYOUT, 11);
        let forked = fork_from(&base, 19, 20, 77);
        let d = d2(&render(&base), &render(&forked));
        assert!(
            d <= TAU_V2,
            "last-twentieth fork moved v2 by {d} > {TAU_V2}"
        );
    }

    #[test]
    fn v2_long_trace_unrelated_content_same_scaffolding_is_far() {
        let a = render(&synthetic_trace(LAYOUT, 11));
        let b = render(&synthetic_trace(LAYOUT, 12));
        let d = d2(&a, &b);
        assert!(d >= 20, "unrelated traces from one harness sit at {d} < 20");
    }

    #[test]
    fn v2_long_trace_same_project_different_session_is_not_within_tau() {
        let (a, b) = same_project_pair(LAYOUT, 5, 21, 22);
        let d = d2(&a, &b);
        assert!(
            d >= 14,
            "same-project pair sits at {d} < 14 for the pinned seed"
        );
        // Across 50 seeds the minimum stays above the v2 threshold. Measured
        // on this generator: mean 19.6, min 10, max 26 -- the shape the
        // calibration table predicts for a pair sharing 40% of its content
        // (J about 0.4 -> expected Hamming about 20, std about 3.5). 64 bits
        // are coarse: a same-project pair lands within tau 8 rarely, not
        // never, which is why the dry run reports the 8-14 band and why
        // author-kind weighting is the documented escalation.
        let all: Vec<u32> = (0..50u64)
            .map(|s| {
                let (a, b) = same_project_pair(LAYOUT ^ s, 500 + s, 1_000 + s, 2_000 + s);
                d2(&a, &b)
            })
            .collect();
        let min = *all.iter().min().expect("fifty seeds");
        assert!(
            min > TAU_V2,
            "same-project minimum over 50 seeds is {min} <= tau {TAU_V2}: {all:?}"
        );
        let mean = all.iter().sum::<u32>() as f64 / all.len() as f64;
        assert!(mean >= 18.0, "same-project mean over 50 seeds is {mean}");
    }

    /// The defect this module's v2 exists to fix, pinned so its removal is
    /// deliberate: under the multiset v1 hash two UNRELATED long traces from
    /// one harness land within `tau_hamming`. On the pilot (2026-09-21) 522
    /// of 696 clustered rows sat in one cluster, every member within
    /// Hamming 10 of its first member, median 6, with only 246 distinct
    /// simhash values among them. Delete this test when `trace_simhash_v1`
    /// is deleted.
    #[test]
    fn v1_regression_pin_unrelated_long_traces_collide() {
        let a = render(&synthetic_trace(LAYOUT, 11));
        let b = render(&synthetic_trace(LAYOUT, 12));
        let d = hamming_distance(trace_simhash_v1(&a), trace_simhash_v1(&b));
        assert!(
            d <= crate::dedup_assign::DEDUP_CONSTANTS_V1.tau_hamming,
            "v1 no longer collides on unrelated scaffolding-heavy traces ({d}); \
             if that is deliberate, delete this pin"
        );
    }
}
