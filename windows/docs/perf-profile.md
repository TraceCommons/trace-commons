# CPU and drawing profile

`windows/scripts/perf/` measures how much CPU the WinUI app's animations
and drawing take, on the test box, with the machine made slower than it
is. Run it before a release, and after any change to the map, the glass
look or anything that animates, and compare against the last run.

It runs on a Debug build (only a Debug build reads `TRACE_COMMONS_SAMPLE`;
a Release build would start a real daemon), on recorded samples, in the
console session, and captures only the app's own windows by handle. It
never signs in, enrolls, accepts terms or shares. See [dev-vm.md](dev-vm.md)
for the box itself. For numbers closer to a release, build Debug with the
compiler's optimisations on (`-p:Configuration=Debug -p:Optimize=true`):
the app's own assemblies are then optimised as Release's are, and the
sample switch still works. It is not a Release build: the `#if DEBUG`
paths (the samples among them) stay, and no Release build has been
profiled. Neither configuration uses ReadyToRun, so in both the runtime
starts methods in its quick tier and recompiles the busy ones, and a
trace of the first seconds shows JIT time.

## The pieces

- `win-perf-run.ps1` (session 0, from SSH) starts `win-perf-drive.ps1` in the
  console session through a hidden `/IT` task, waits, prints its log, and
  reports whether the app's own state on the machine changed. It also runs
  `wpr.exe` when the plan asks for a trace, because `wpr` needs the
  elevated token the console task does not have.
- `win-perf-drive.ps1` follows a plan from `plans/` (the step list is at the
  top of the script) and writes `<OutDir>\<Tag>.perf.tsv`, one line per
  measurement window, and `<Tag>.drive.log`.
- `PerfProbe.cs` holds the Win32 calls: the throttled launch, per-thread
  CPU, the machine's busy time, DWM's CPU and the UI-thread round trips.

```bash
windows/scripts/win-exec-az.sh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\src\trace-commons\windows\scripts\perf\win-perf-run.ps1 -Exe "C:\src\trace-commons\windows\src\TraceCommons.App\bin\x64\Debug\net8.0-windows10.0.19041.0\win-x64\TraceCommons.exe" -Tag map-full -Throttle full -Plan C:\src\trace-commons\windows\scripts\perf\plans\map.txt'
```

A whole matrix (every plan at every level) takes about 40 minutes on a
`D4s_v3`, so run it as a job (`win-exec-az.sh --job`).

## How the machine is made slower

The app is started suspended, put in a job object, and only then let run,
so the limit holds from its first instruction. The job is created with
kill-on-close, so the app never outlives the run.

| Level | Cores (affinity) | CPU cap (share of the whole 4-core box) | Models |
|---|---|---|---|
| `full` | all 4 | none | the box as it is |
| `2core` | 0-1 | none | a dual-core machine at full speed |
| `2core-half` | 0-1 | 25% (one core's worth) | a dual-core machine at half speed |
| `1core-half` | 0 | 12.5% (half a core) | the floor: one slow core |

The cap is Windows' hard cap (`JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP`): the
app gets its share each scheduling interval and then waits, so it models
fewer cycles, not a slower clock. The harness itself is pinned to core 3.

What it does not slow down: DWM (`dwm.exe`) and the GPU driver are shared
system processes outside the job. The box has no GPU, so both WinUI's own
compositor (`dwmcorei.dll`, inside the app) and DWM render with WARP, on
the CPU: a machine with any GPU spends less, a machine with a slow CPU and
no GPU spends more in DWM than the box shows.

## What a line says

| Column | Meaning |
|---|---|
| `app_ms` | the app's CPU in the window, all threads (10 s of one core is 10000) |
| `ui_ms` | the UI thread's share of it |
| `other_ms` | the busiest other thread |
| `dwm_ms` | DWM's CPU in the same window (it composes the app's frames) |
| `machine_ms` | every core's busy time, everything on the box |
| `harness_ms` | the driver's own CPU, to subtract |
| `dwm_frames` | DWM's composed-frame count; on the box's Hyper-V display it does not advance, so it reads 1 or 0 |
| `ping_*` | WM_NULL round trips to the window every 50 ms from another thread: a message waits until the UI thread pumps, so this is how long a click or a key waits to be seen. `over50` counts trips over 50 ms |
| `ws_mb`, `private_mb` | working set and private bytes at the end of the window |
| `frames_by_surface` | each drawn surface's frames per second, mean draw-handler time and longest gap between frames, from the app's look log (see below) |
| `other_threads_by_start_module` | the other threads' CPU grouped by the module they started in: `ntdll` is the thread pool (WARP's rasteriser works there), `dwmcorei` WinUI's compositor, `coreclr` the GC and JIT |

Every launch sets `TRACE_COMMONS_LOOK_LOG` to `<OutDir>\<Tag>.look.log`. The
app's `LookFrames` writes one line a second for each surface it draws there
(`stage` for the first-run stage, `map` and `map-still` for the Traces
map's two layers, `home-glyphs` for Home's hero glyphs). The handler's time
leaves out the rasterising Direct2D does after the handler returns, so the
frame rate, not the handler time, shows a frame that is too slow. Nothing
is timed unless the variable is set.

`respond <label> <Name>` clicks a control and times until the window's
pixels change, read with `PrintWindow`; each read takes tens of
milliseconds, so it is coarse and its floor is the read itself.

No UI Automation call is made inside a measurement window: those run on
the app's UI thread and would be measured.

## Finding the functions

`wpr start [profiles]` and `wpr stop <label>` in a plan record an ETW trace
(`CPU` by default) to `<OutDir>\<Tag>-<label>.etl`. Open it in Windows
Performance Analyzer, or aggregate the sampled stacks with the
`Microsoft.Windows.EventTracing.Processing` package in a throwaway console
project on the box (never in this repository). Use `CPU` alone: the
`DotNET` profile walks a stack on every runtime event and raised
the app's CPU by about half in the traced windows. Without it, most managed frames still
resolve; frames the runtime compiled after the trace began show as `?`.

## Plans

- `map.txt`: the Traces map on `armedFolder`, the station's first turn
  (0-30 s), the cached turn with the shimmer (30-60 s), minimised,
  restored, hidden to the tray, and a second launch with animations off.
- `surfaces.txt`: Home, a pointer over the main tabs, Insights and its tab
  bar, Skills Home (`skillsHome` extra), the Traces tree, glass hover and a
  switch pressed repeatedly, minimised, and the tray flyout.
- `firstrun.txt`: the first-run stage on its Traces, Uses and Join steps.
- `rail.txt`: the edge rail, switched on in Settings (sample state only)
  and opened.

The driver turns the system's client-area animation setting on for the
run and puts back what it found; the log records both.
