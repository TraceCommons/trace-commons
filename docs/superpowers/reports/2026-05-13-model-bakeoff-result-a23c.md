# Bake-off report (2026-05-14T02:31:40.164765905+00:00)

- corpus: sha256:fe461f4aabfccf9d53c0e5261db7100598320d2c55a1c74fef95ad5d54681b0a
- manifest: sha256:2e360df9449d81d664caeb0e17ed893ccb28e5998604c4caafd1aa46a13fd0f0
- decision-rule version: 1
- ctx_max_tokens: 4096
- determinism gate: 0.00001

Winner: qwen3-8b-base

| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| llama-3.1-8b-instruct | 0.119744 | 0.124916 | 0.127322 | 288.547 | 0.000e0 | LlamaCommunity | 8 | true |
| qwen3-8b-base | 0.235000 | 0.141600 | 0.009901 | 245.492 | 4.163e-17 | Apache2 | 8 | true |
| qwen3.6-27b-dense | 0.275922 | 0.139843 | 0.012519 | 118.233 | 1.249e-16 | Apache2 | 27 | true |
| gemma-4-31b | 0.054500 | 0.173408 | 0.201885 | 200.448 | 1.110e-16 | Apache2 | 31 | true |
