# Bake-off Result Notes (2026-05-14, A2.6)

Companion to `2026-05-14-model-bakeoff-result-a26.{json,md}`.

## What we tested

A2.6 retrofits the A2.3c / A2.4 bake-off with a different novel slice.
The novel slice is drawn from `jedisct1/agent-traces-swival` — 33.7k
OSS security-audit traces (MIT-licensed, ungated) captured from the
Swival open-source agent. Each row's `proof` + `fix_outline` + first
~1000 chars of `source_code` are joined as prose and length-filtered to
200-2000 words, sampled to 300 with a deterministic seed. The duplicate
slice (Wikipedia article intros) and paraphrase pipeline (Qwen3-4B-Base
back-translation, batched) are reused unchanged from A2.4 so the only
moving variable is the novel-slice corpus shape. The candidate set is
identical to A2.3c / A2.4: Llama-3.1-8B-Instruct, Qwen3-8B-Base,
Qwen 3.6 27B Dense, Gemma 4 31B Base. Hardware is Lambda H100 SXM5 80GB.

## What we found

Partial results as of 2026-05-14 18:01 UTC; Gemma 4 31B Base is still
scoring. A clear size pattern is visible in the three completed
candidates: the two 8B candidates (Llama-3.1-8B-Instruct at AUC 0.3425,
Qwen3-8B-Base at AUC 0.2431) flunk the 0.5 threshold, while the 27B
dense candidate (Qwen 3.6 27B Dense) jumps to AUC 0.9363 — a roughly
3.5x leap over its 8B siblings on the same corpus, paraphrase pipeline,
and decision rule. Outcome 1 (at least one candidate AUC > 0.5) has
already fired via the 27B result. The swival agent-traces novel slice
shifted Llama-3.1-8B-Instruct from A2.4's 0.240 to 0.343 (a real but
sub-threshold improvement) and left Qwen3-8B-Base flat (0.207 → 0.243).
The Qwen 3.6 27B and Gemma 4 31B candidates have no comparable A2.3c /
A2.4 baseline — both runs aborted on model load under candle 0.10.2;
A2.6 is the first bake-off where mistralrs lets the larger candidates
actually produce AUCs.

## Why this matters

Outcome 1 firing validates the corpus-design hypothesis at sufficient
model size: a security-audit novel slice paired with Wikipedia-intro
duplicates produces a perplexity signal that crosses 0.5 once the model
is large enough to internalize the duplicate-slice distribution
tightly. A2.5's pilot-launch perplexity floor of 0 stops being a safe
default — A2.7 (re-enable perplexity floor against the
swival-calibrated distribution) is now an active follow-up rather than
a contingency, and Phase A.5 (perplexity-replacement metric) is
deferred. Operationally, this also means production deployment can no
longer assume an 8B-class server is sufficient for the novelty signal:
the 27B candidate is the smallest one that crossed, and inference at
that size requires a 1xH100 80GB minimum (peak 57019 MiB observed
during scoring).

## Cost + time

Partial: cost so far is approximately $21 across roughly 7 hours of
H100 SXM5 time covering the three completed candidates
(Llama-3.1-8B-Instruct 4989s, Qwen3-8B-Base 5962s, Qwen 3.6 27B Dense
~3h17m precise figure lands with the report JSON). Gemma 4 31B Base is
expected to add another ~$15-20 over 3-4 more hours. Total run: ~$35-45
across ~9-11 hours. Region <TBD>; Lambda hourly rate <TBD>;
paraphrase-pipeline time <TBD> (rolled into corpus build, not the
scoring loop).

## Lessons

<TBD: deviations from spec, surprises in the data, any candle /
mistralrs / corpus-builder issues, anything the next operator should
know before re-running. To be filled when Gemma 4 31B completes and
the report is finalized.>
