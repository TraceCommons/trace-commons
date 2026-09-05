# Compute contribution in the macOS shell

Compute is an independent sidebar destination. It can be selected before trace
roots are declared or trace enrollment/onboarding is complete. Existing trace
contributors still start the watcher as before; a fresh installation is refused
by Rust's existing roots gate before any watcher is started. Selecting Compute
does not enroll, discover trace sessions, or change trace consent.

The application owns `ComputeModel` and `MainWindowNavigation` above window
lifetime. `ComputeModel` uses a dedicated serial queue for controller I/O, polls
actual snapshots, and reads all compute wording from Rust. Missing/unsupported
snapshots disable actions. RAM is described as a scheduling allowance, never an
enforced ceiling or a measured footprint.

The shipping `tc_compute_open` constructor remains unavailable. This shell does
not opt into a development worker constructor, ship a worker, or contact a pool.
No device eligibility, battery/thermal/memory-pressure adapter, signing policy,
MLX asset packaging, or real-workload evidence is supplied by this UI slice.
Those controls must be implemented before enabling packaged compute.

## Quit and ownership

Every Quit entry point passes through `AppDelegate`. After confirmation, one
`QuitCoordinator` requests controller shutdown off the main thread (15-second
controller timeout). Repeated requests share that stop operation. The outer
17-second deadline replies false to AppKit and leaves the app running; it does
not cancel or free the controller. Late success cannot unexpectedly quit the app.

Only an explicit `worker_stopped` result permits termination. This is process
evidence, separate from `drain_outcome`; forced or unacknowledged stops must not
be described as graceful handoffs. A false/unknown result retains ownership and
routes the user to Compute with the shared refusal text. Controller shutdown is
terminal: if it succeeds after Quit was already refused, the model can reopen a
fresh paused controller only after confirming the old worker stopped. Resume
still requires an explicit action. No stop or close occurs when switching tabs
or closing the window.

The existing trace daemon cleanup remains on `willTerminateNotification`, after
compute permits termination. Unexpected OS termination and sleep do not go
through this cooperative Quit guarantee.

## Verification

`ComputeNavigationTests` exercises independent startup with real temporary state,
the existing roots refusal, unchanged trace onboarding gates, shared copy on
invalid compute settings, safe idle shutdown, and CPU rendering of the same
content the native scroll view contains. Set `TC_COMPUTE_TEST_RENDER_PATH` to an
explicit PNG path when running that suite to inspect the unavailable surface.

`QuitCoordinatorTests` covers cancelled/repeated Quit, timeout with late success,
and unconfirmed stop. `ComputeExportTests` covers the actual C ABI, shared copy,
strict snapshot decoding, consent persistence, and concurrent handle close.
These are not evidence of a signed installation, a real worker drain, or native
power/sleep behavior. SwiftUI accessibility enumeration was not available in the
unit-test host, so navigation checks operate on the app's real navigation/model
objects; they do not claim an automated native mouse-click test.
