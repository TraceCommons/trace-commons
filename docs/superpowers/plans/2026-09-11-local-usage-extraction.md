# Local native usage extraction

The Insights usage extractor reads only explicitly supplied bytes from a single
JSONL file. It never discovers sibling sessions or Claude delegated transcripts,
performs network calls, or modifies contribution adapters. Its observations are
source declarations, not billing records or verified serving identities.

Supported synthetic fixture contracts:

- Codex `event_msg` / `payload.type = token_count` /
  `payload.info.total_token_usage`: input, cached input, output, reasoning output,
  and total tokens, all required nonnegative u64 integers. The latest monotonic
  cumulative snapshot is retained; repeated snapshots and `last_token_usage`
  are not added. Cached input and reasoning output are subsets. A counter reset
  makes the aggregate unavailable, because session continuity is unproven.
- Claude Code `assistant` / `message.usage`: input, cache-read input,
  cache-creation input, and output, all required nonnegative u64 integers.
  Separate cache categories remain separate. Repeated `message.id` records use
  the latest monotonic usage snapshot, including tool-only content. Missing IDs,
  regressing duplicates, and changed model declarations for one message refuse
  the aggregate. Cache-duration splits and service modifiers are not priced.

`usage_records` counts candidate native records, including repeated snapshots;
`complete_records` counts candidates with structurally valid numeric usage.
These are record coverage, not API-call, task, or session completeness. An
incomplete candidate makes the aggregate unavailable even if others are valid;
absence never becomes zero. Arithmetic overflow also makes totals unavailable.

Model labels are source-declared, restricted to a bounded ASCII identifier
alphabet and 96 bytes, and capped at 32 distinct labels. Omitted labels are
flagged. This cannot guarantee that a syntactically valid label is a real model.
There is deliberately no allocation of cumulative tokens to the final model:
Codex snapshots across model switches cannot prove that attribution. Per-model
usage segmentation and pricing remain follow-on work.

Tests use synthetic records only; no private transcripts are copied into tests.
Historical format variants missing required accounting fields remain unknown.
