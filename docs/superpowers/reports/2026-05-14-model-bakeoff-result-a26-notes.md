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

<TBD: AUC summary across all 4 candidates, which branch of the
three-outcome decision tree fired, and how the swival corpus shifted
each candidate's AUC relative to A2.3c (OASST2 / boilerplate) and A2.4
(OASST2 / wiki intros).>

## Why this matters

<TBD: tie to A2.5's "perplexity floor = 0 at pilot launch"
recommendation; conditional A2.7 PR (floor-recommendation update) firing
or not firing; whether Phase A.5 (perplexity-replacement metric) stays
on the roadmap or gets parked / closed.>

## Cost + time

<TBD: total elapsed wall-clock from instance launch to termination;
Lambda hourly rate; total spend; per-candidate scoring time;
paraphrase-pipeline time; region used.>

## Lessons

<TBD: deviations from spec, surprises in the data, any candle /
mistralrs / corpus-builder issues, anything the next operator should
know before re-running.>
