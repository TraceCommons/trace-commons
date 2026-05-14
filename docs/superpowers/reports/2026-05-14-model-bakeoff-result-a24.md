# Bake-off report (2026-05-14T05:17:10.475869433+00:00)

- corpus: sha256:7f4349686db668d081f0d9ebfbe682e803bd888fff61705c527c975db631d718
- manifest: sha256:2e360df9449d81d664caeb0e17ed893ccb28e5998604c4caafd1aa46a13fd0f0
- decision-rule version: 1
- ctx_max_tokens: 4096
- determinism gate: 0.00001

Winner: llama-3.1-8b-instruct

| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| llama-3.1-8b-instruct | 0.240022 | 0.124916 | 0.021226 | 316.222 | 0.000e0 | LlamaCommunity | 8 | true |
| qwen3-8b-base | 0.206522 | 0.141600 | 0.025124 | 270.651 | 4.163e-17 | Apache2 | 8 | true |
| qwen3.6-27b-dense | 0.264117 | 0.139843 | 0.022289 | 126.092 | 1.249e-16 | Apache2 | 27 | true |
| gemma-4-31b | 0.184867 | 0.173408 | 0.032828 | 213.945 | 1.110e-16 | Apache2 | 31 | true |
