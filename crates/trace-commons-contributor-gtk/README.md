
## Local Insights

The default launch window opens Insights before resolving contributor state or
starting a worker. The Contributions action enters the existing roots and
onboarding flow; those gates remain required for contribution. An initialized
contribution window also includes an Insights destination.

Choose one Codex rollout or trajectory file to analyze it without saving.
**Re-read and save** explicitly reads that file again and saves derived
observations to the same owner-only Insights store used by the CLI. Refresh
saved insights lists dated snapshots; **Show evidence** displays source digests,
coverage, analyzer/rubric attribution, and unknown values. It does not re-read
sources. Saved snapshots support user-reported task assessments and deletion;
original files are never deleted. Neither operation enrolls or uploads.

IO runs serially off the GTK thread. Cancel suppresses a pending result but
keeps controls disabled until that request finishes, so repeated cancellation
cannot enqueue unbounded work. Closing the window or application suppresses
late UI updates. Already-started saves/deletions may finish; refresh history to
check. No automatic source watching or model ranking is provided.

Linux display qualification (run each test in its own process under Weston):

```sh
RUSTFLAGS='-D warnings' cargo test --locked --lib \
  ui::insights::tests::account_free_view_analyzes_saves_explains_deletes_and_ignores_closed_results \
  -- --exact --ignored --test-threads=1
RUSTFLAGS='-D warnings' cargo test --locked --bin trace-commons-shell \
  insights_startup_tests::first_run_local_window_does_not_create_contributor_state \
  -- --exact --ignored --test-threads=1
cargo run --locked --bin trace-commons-shell -- \
  --start-page insights --exit-after-realize --realize-seconds 3
```

The separate-workspace unit suite does not replace the Linux Weston UI smoke.
On macOS, GTK requires initialization on the OS main thread, so its display
scenarios cannot run inside the Rust test harness; the actual application smoke
command above runs on the main thread. Dates use GLib local time and locale
formatting; metric counts retain exact ungrouped decimal values.
