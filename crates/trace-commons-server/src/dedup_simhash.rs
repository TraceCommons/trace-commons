// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic 64-bit token simhash over a trace's canonical text, for
//! cross-trace duplicate clustering. Word-shingle features, FNV-1a hashed for
//! build-stable reproducibility. Pure: no I/O.

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

pub fn trace_simhash(canonical_text: &str) -> u64 {
    let toks = tokens(canonical_text);
    if toks.is_empty() {
        return 0;
    }
    // Features = overlapping 2-token shingles (fall back to unigrams for a
    // single token) so word-order matters and light rewords stay close.
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
    }
}
