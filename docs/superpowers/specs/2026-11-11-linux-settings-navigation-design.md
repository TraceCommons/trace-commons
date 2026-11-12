# Linux Settings navigation

The authorized first slice brings the existing GTK Settings controls into the native macOS section order. It does not claim complete Settings or onboarding parity.

## Approach

Keep one scrollable body and add grouped navigation. A page-per-section stack would hide adjacent context and differ from macOS; leaving the long column alone would not provide direct navigation. The selected approach reuses the existing cards and action handlers. Wide windows use a sidebar; below 760 logical pixels the same selection is a section menu above the body. No dependencies, notices, or daemon changes.

Existing controls form Account, Watching, Tools, Projects, Public profile, Change log, Private AI, Startup & notifications, and Redaction witness. Shared monitor settings navigation copy names every section. Unsupported sections are not fabricated. Connection evidence and pause remain together under Watching; account recovery and login changes move under Account. Timing controls stay beside the watcher. Public profile keeps its existing state-dependent heading.

Keyboard and assistive technology use native ListBox and MenuButton controls; the chooser uses the existing Settings name. Section choices scroll to the requested group, clamped to valid scroll bounds. Resizing preserves selection and entered field values; it never rebuilds controls or sends settings writes.

## Separate queued onboarding migration

GTK's Welcome / Connect / Consent / Scan / Watch / Done is not macOS's Welcome / Tools / Private AI / Traces / Uses / Terms / Join. Port FirstRunState, FirstRunNavigation, FirstRunPlan and runner contracts before changing page order. Preserve terms acceptance, required scopes, enrollment before scoped writes, readiness before grants, Ask-rule ordering after grants, browser login/provider environment policy, retry state, and completion only after successful writes. Keep explicit scanner recovery available. This is outside the structural Settings slice.

## Verification

Test section order/copy, scroll clamping, selection across wide/narrow layouts, actual GTK navigation signals and preservation of a field value across resize. Run the full GTK suite once after implementation using RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0, plus format and clippy. Display-dependent tests are separately identified and run on a real available GTK display; Linux runtime evidence remains distinct from macOS compilation.
