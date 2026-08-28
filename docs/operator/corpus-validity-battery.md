# Corpus validity: the trivial-measure battery

Sub-project B of the gate-validity program
(`docs/superpowers/specs/2026-08-28-gate-validity-program.md`). Closes the
construction half of #204; the operating-point half of #205 belongs to
sub-project E and is untouched here.

## What went wrong

The bake-off that selected the production perplexity scorer scored candidates
with `discrimination_auc(novel, duplicate)` on
`scripts/operator/fixtures/corpus-a26.tar.zst`
(sha256 `46e0eef8a52e309ce695ad20d1e242ce43eb210c11e02764beeaf7fa3d341bb5`).
`scripts/operator/build-agent-traces-corpus.py` built that corpus's novel slice
from agent traces and reused its duplicate and paraphrase slices verbatim from
a separate `corpus-wiki.tar.zst`. Novel and duplicate came from different
source populations, so every property that tracks source separated the classes.

Recomputed with the repository's own tie convention
(`discrimination_auc`, `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs:18`
— ties 0.5 each, Mann-Whitney U, empty inputs 0.5):

| measure | novel vs duplicate | novel range | duplicate range |
|---|---:|---|---|
| paragraph count | **1.000000** | 7..163 | 1..1 |
| line count | 0.998244 | 29..762 | 1..129 |
| distinct word count | 0.993128 | 160..901 | 103..632 |
| UTF-8 byte count | 0.991117 | 1,628..221,623 | 1,120..8,100 |
| whitespace word count | 0.985883 | 222..1,994 | 200..1,301 |
| mean word length | 0.983622 | 5.44..189.89 | 4.30..8.09 |

`qwen3.6-27b-dense`, the selected candidate, is archived at 0.9362666666666667.
All six beat it. Paragraph-count support is disjoint, so there is no stratum in
which to hold format constant and ask about novelty: the question the bake-off
intends to answer is not identifiable on that corpus, by any analysis.

The paraphrase slice does not repair it. Holding the source constant removes
the format confound and leaves a length confound just as strong — byte count
scores 0.996106 on `original vs paraphrase`, because 299 of the 300
paraphrases are shorter than their original.

## The battery

Six preregistered no-model measures, run through the repository's own
`discrimination_auc`:

`paragraph_count`, `line_count`, `distinct_word_count`, `utf8_byte_count`,
`whitespace_word_count`, `mean_word_length`.

Preregistered means fixed before a corpus is built. Adding a measure after
seeing a corpus's numbers, or dropping one that fails, defeats the point.

A corpus is **admissible** only if every measure lands within `|auc - 0.5| <=
ceiling` on both `novel vs duplicate` and `original vs paraphrase`. The default
ceiling is 0.15. Empty slices are never admissible: `discrimination_auc`
returns 0.5 for them, which is the right answer to "which class is higher" and
the wrong answer to "is this a corpus".

`utf8_byte_count` is also reported at the top of every result as the length
covariate, per the guardrail lifted out of #199: if a score's AUC matches the
covariate's to two decimals, the score is measuring length.

### Running it

```bash
cargo build -p trace-commons-server --bin trace-commons-gate-calibrate
./target/debug/trace-commons-gate-calibrate corpus-validity \
  --corpus path/to/corpus.tar.zst \
  --ceiling 0.15 \
  --out corpus-validity.json
```

Non-zero exit means inadmissible. `--audit-only` reports the verdict and exits
0; use it to audit a corpus already known to be bad, never from a builder.

### Where it is enforced

1. **On construction.** `build-agent-traces-corpus.py` packs to a temporary
   path, runs the battery against it, and only then copies to `--out`. A
   missing `trace-commons-gate-calibrate` produces no corpus at all —
   fail-closed, because a corpus nobody checked is exactly what #204 is about.
2. **In CI.** `crates/trace-commons-server/tests/corpus_trivial_measures.rs`
   pins the six measure definitions and pins the battery's verdict on the
   known-bad A2.6 fixture. A battery that cannot reproduce a known-bad
   corpus's failure is not evidence about a good one, so that reproduction is
   a permanent regression test rather than a one-off audit.

## The corrected construction

Both slices now come from one population and differ only in novelty:

```
novel[i]     = an agent trace
duplicate[i] = a transformed version of THAT SAME trace
```

`paraphrase.jsonl` carries the same pairs, so the paraphrase metric and the
discrimination metric are computed over one construction rather than two
unrelated ones.

Two transforms:

* `shuffle-paragraphs` (default) — permutes the trace's paragraphs. Model-free,
  deterministic, no GPU, no network.
* `external` — pipes through `--transform-cmd`, a paraphrase or
  back-translation helper (`scripts/operator/bakeoff_paraphrase.py` implements
  the contract).

`--length-band` (default 0.10) rejects any pair whose duplicate differs from
its original by more than that relative word count, and the build fails rather
than emitting a length-confounded corpus. This is enforced rather than assumed
because assuming it is what produced the A2.6 paraphrase slice.

### The reference corpus

Built over the same 300 agent traces the A2.6 corpus used, so the novel slice
stays comparable to the archived bake-off:

```bash
python3 scripts/operator/build-agent-traces-corpus.py \
  --novel-corpus scripts/operator/fixtures/corpus-a26.tar.zst \
  --transform shuffle-paragraphs \
  --count 300 \
  --validity-binary ./target/debug/trace-commons-gate-calibrate \
  --out corpus-b1.tar.zst
```

Deterministic and reproducible from a file already in the tree. `pack_tarball`
normalises entry metadata (zero mtime, zero uid/gid, fixed mode), so the
archive does not carry the build host or the wall clock; without that the
digest changed on every run and could not have been cited here.

Slice digests, which are over file bodies and so are independent of the zstd
compressor used:

```
novel_sha256      sha256:e9a17becfa98c50a3224d73da5fbb202eeaa220d9827431cd2ad79c6ac317b4c
duplicate_sha256  sha256:ff73a62e28d167f756a791e900cfd0a25b876a41297e0baa2d874db5839ae51f
paraphrase_sha256 sha256:f2cff54951294fb680c4553bed8c6fc5d766be47fc6d7e41623c238a3f23330b
```

`novel_sha256` is byte-identical to the A2.6 corpus's, which is the check that
the novel slice really is the same 300 traces.

Tarball sha256, built with the `zstd` CLI:
`ec6d55c3240d30f8372858bfa1bf3227d954e577a4b95470ff54b79d6ba5117c`

Battery result, both pairs:

| measure | AUC | deviation from 0.5 |
|---|---:|---:|
| paragraph_count | 0.500000 | 0.000000 |
| line_count | 0.500022 | 0.000022 |
| distinct_word_count | 0.500000 | 0.000000 |
| utf8_byte_count | 0.500000 | 0.000000 |
| whitespace_word_count | 0.500000 | 0.000000 |
| mean_word_length | 0.500000 | 0.000000 |

Verdict: ADMISSIBLE.

## What this does not establish

Three things, stated plainly because the temptation on reading a table of 0.5s
is to conclude the problem is solved.

**Passing the battery is necessary, not sufficient.** It says six named
structural properties do not separate the classes. It says nothing about the
seventh that nobody wrote down.

**The default transform passes by construction.** `shuffle-paragraphs` moves no
bytes, so byte count, word count, distinct word count, paragraph count and mean
word length are *identical* between the slices and land at exactly 0.5 by
arithmetic, not by luck. That is the right shape for a redundancy gate — a
duplicate that really is the same content — and it is a structural control, not
a semantic-difficulty benchmark. `line_count` at 0.500022 is the one measure
that moves at all, and only on 2 of the 300 traces: both contain a run of three
or more newlines, which splits into an empty paragraph block, and when the
permutation lands that empty block last the trace ends in a trailing newline
pair, which `lines()` counts as one line fewer. The battery earns its keep on the `external`
transform, where the text genuinely changes and a truncating paraphraser would
otherwise reintroduce exactly the length confound #204 found.

**No model number was measured on the corrected corpus.** Re-running the
bake-off is explicitly out of scope for sub-project B, and no GPU or scorer was
touched here. The `qwen3.6-27b-dense` selection is retained as
**unproven-but-inherited**: the archived 0.936 is a source-format detector's
score and should not be cited as evidence for the model, and there is as yet no
replacement number to cite instead. "Why this model" is formally unanswered.
Deriving it belongs to sub-project E, alongside the floor re-derivation #205
asks for.
