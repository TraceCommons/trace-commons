# GTK live Overview

Approved direction: user requested work on macOS UI parity; start with Insights content, then Home/shell and Traces. This slice connects the existing daemon weekly Overview to GTK while preserving standalone saved-file analysis.

Connected GTK windows request insights_week through their existing App worker; standalone Insights retains its current local service and never starts or configures a daemon. Render core WeekOverview when live enabled/readable, plus weekly work totals, spend by model and trends where supplied. Keep unknown values distinct from zero and partial list-price spend distinct from actual charges. Use the existing shared copy table and macOS formatting semantics. No new notices, dependencies, activation settings, network providers or uploads.

Use generated Insights components. Preserve all existing tabs, drill-down reachability and saved-data fallback. Live figures must not open saved-feed drill-downs masquerading as their evidence. Week selection requests ISO weeks and protects against stale callbacks. Disabled/unreadable/malformed live response falls back honestly to saved content without retaining stale live cards. All IO stays on the existing worker.

Usage, credit, subscriptions and recaps can be added only when their availability and action semantics can be preserved; otherwise explicitly record as next slice rather than showing placeholders or invented totals.

Validation: pure formatting/availability tests, request and stale-response guards, GTK compile/test/format/CI clippy, generated token check. Full app Linux runtime qualification remains a separate gate. Prior styling PR2083 has an independently handled tab-row regression; incorporate its fix before completion.
