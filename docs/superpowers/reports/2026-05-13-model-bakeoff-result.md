# Bake-off report (2026-05-13T16:27:11.402543847+00:00)

- corpus: sha256:8acb0be339b2da278986c389700884b23a92dafc85e41c6549d963b550938660
- manifest: sha256:6870f59aec03472180ae86c569933bade4a7b1abe9105d4dfd5587921e0814bf
- decision-rule version: 1
- ctx_max_tokens: 4096
- determinism gate: 0.00001

Winner: qwen3-8b-base

| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| llama-3.1-8b-instruct | 0.105456 | 0.131787 | 0.113698 | 99.826 | 1.110e-16 | LlamaCommunity | 8 | true |
| qwen3-8b-base | 0.719956 | 0.196551 | 0.111317 | 93.253 | 4.832e-13 | Apache2 | 8 | true |
