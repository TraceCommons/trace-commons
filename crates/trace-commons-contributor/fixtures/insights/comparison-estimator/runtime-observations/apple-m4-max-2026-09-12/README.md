# Exact candidate evaluation runtime observations

This directory records a finite runtime and memory observation of commit
`331486223caec66b8f3a06d4e717ed101d816a74` on an Apple M4 Max with 128 GiB
of memory and 16 CPU cores. It qualifies only the four recorded count
compositions on this machine. It is not a worst-case bound or product
admission result.

A complete supported evaluation validates one frozen count input, computes all
six exact intervals for two cohorts by three outcomes, combines each outcome's
two intervals, and applies the width and zero-boundary decision rules. The
measurement excludes source parsing, task eligibility, persistence, service,
and UI work. The `imbalanced_suppressed` control exits at the minimum cohort
support check and does not compute intervals.

Each build/case pair has run `0` as an excluded warmup and runs `1` through `3`
as fresh-process measurements. The JSON field `candidate_elapsed_nanos` covers
only the candidate evaluation call. The `.time` files come from macOS
`/usr/bin/time -l -p`; its process wall time has 10 ms display granularity and
its maximum resident set size is in bytes. Peak RSS includes the Rust test
harness process, not only allocations made inside the candidate call.

Median observations:

| Build | Composition | Candidate time | Process wall | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| Debug | balanced boundary, 128/128 | 40.291 ms | 0.05 s | 13,664,256 B |
| Debug | balanced interior, 128/128 | 88.929 ms | 0.10 s | 13,680,640 B |
| Debug | supported imbalanced, 254/2 | 236.711 ms | 0.25 s | 13,713,408 B |
| Debug | suppressed control, 255/1 | 0.018 ms | 0.01 s | 12,189,696 B |
| Release | balanced boundary, 128/128 | 5.323 ms | 0.01 s | 10,338,304 B |
| Release | balanced interior, 128/128 | 11.034 ms | 0.02 s | 10,371,072 B |
| Release | supported imbalanced, 254/2 | 20.322 ms | 0.03 s | 10,338,304 B |
| Release | suppressed control, 255/1 | 0.000208 ms | 0.01 s | 9,928,704 B |

`summary.json` contains all three measured values and their median, minimum,
and maximum. `SHA256SUMS` binds every evidence and driver file. Run
`measure.sh DEBUG_TEST_BINARY RELEASE_TEST_BINARY OUTPUT_DIRECTORY` from a
checkout of the measured commit to reproduce the raw observations. The script
refuses an existing output directory because the Rust writer also publishes
each observation without replacement.
