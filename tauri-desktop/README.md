# Trace Commons Tauri Desktop

Cross-platform Tauri app backed by the existing Rust contributor core. Swift,
WinUI, and GTK applications remain unchanged. Tauri source/build success does
not establish native OS or release parity; current gates live in
[ARCHITECTURE.md](./ARCHITECTURE.md).

## Run from repository root

```bash
./tauri-desktop/scripts/dev.sh
./tauri-desktop/scripts/build.sh
./tauri-desktop/scripts/build.sh --release
./tauri-desktop/scripts/start.sh
./tauri-desktop/scripts/start.sh --no-build
```

The development script runs Vite on port 1420 with Tauri reload. Build scripts
install frontend dependencies from the frozen lockfile. Contributor state
uses the shared core directory, including `TRACE_COMMONS_CONTRIBUTOR_DIR`.
Desktop launch does not require PostgreSQL or Docker.

Development builds use bundle identifier
`ai.tracecommons.tauri.prototype`; release packaging currently declares
`ai.tracecommons.desktop`. Confirm final identity and any state migration
before promoting packages. See [ARCHITECTURE.md](./ARCHITECTURE.md).
