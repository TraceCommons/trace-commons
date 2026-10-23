# Portable CI with Dagger

The root `dagger.json` pins Dagger **0.21.11**. Its Python module in `.dagger/`
exposes six checks. The same `dagger check` command runs locally and in Dagger
Cloud Checks. The pipeline makes no LLM calls and needs no model/API credentials.

## Local use

Install Dagger 0.21.11 from its official release and start a Docker-compatible
container engine. On macOS, Docker Desktop or Colima works (`colima start
--cpu 8 --memory 16 --disk 100`). Then, from this checkout:

```sh
dagger version
dagger check -l                 # discover check names
dagger check                    # run all six concurrently
dagger check '*:format' '*:browser' '*:safety'   # fast pre-push checks
dagger check '*:build'           # select one lane
dagger --progress=plain -v check # verbose operation and cache evidence
scripts/ci/dagger-cache-demo.sh  # run identical checks twice and retain logs
```

The demo stops immediately if either run fails. Logs and wall-clock timings go
to `.local/dagger-runs/<timestamp>/`, which is excluded from pipeline inputs.
`cached_vertices` counts log lines containing CACHED, including setup vertices;
inspect the actual Cargo/test operations or completed check functions to establish
that work was reused. A cached test result means the previous result was reused;
it does not mean the test process ran a second time. Startup, source hashing,
and cache lookup still take time.

If an old Docker Desktop configuration names a missing
`docker-credential-desktop`, Docker pulls fail even while Colima is healthy.
Use a dedicated empty Docker config for these public images, preserving the
existing config and explicitly retaining the current runtime endpoint:

```sh
mkdir -p .local/dagger-docker
printf '{}\n' > .local/dagger-docker/config.json
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
export DOCKER_CONFIG="$PWD/.local/dagger-docker"
dagger check
```

## Coverage

| Check | Existing CI checks reproduced |
| --- | --- |
| `format` | Root workspace `cargo fmt --all -- --check` |
| `safety` | Release credential-store and staging-feature guards with mutation self-tests, macOS channel plists, staging markers, Sparkle appcasts, deploy rendering, backup/restore shell fixtures, offline privacy projections, OpenCode qualification pin checks |
| `browser` | Wallet browser states, step-up page script, AASA HTTP contracts |
| `build` | Default server/ingest bins and contributor, plus contributor/FFI and server/ingest staging variants |
| `clippy` | Root workspace all targets and the existing three staging variants, with CI's five unchanged lint allowances |
| `tests` | Whole workspace, the existing staging channel test selection, real Secret Service round trips, wallet SDK and production CSP checks |

Rust is pinned to **1.99.1**, Node to **24.22.1**, and Playwright to **1.63.0**.
Every Cargo invocation retains `RUSTFLAGS=-D warnings` and `CARGO_INCREMENTAL=0`.
Checks run as an unprivileged `ci` user, matching GitHub Actions: permission-refusal tests must not inherit root's write-permission bypass. Browser binaries are installed in a shared readable directory. Debug information is disabled to keep these validation artifacts smaller.
No application dependency or Cargo lockfile changes are needed.

This is the first portable lane, not full CI parity. Existing GitHub Actions
remain authoritative for PostgreSQL suites, native macOS/Windows/GTK, MSRV,
other feature matrices and TLS selfchecks, dependency licenses/advisories,
release smokes, pipeline qualification, mark generation, and the privacy shim.
Workspace database tests still self-skip without their database URLs; a green
Dagger run proves no PostgreSQL coverage. The deploy-render fixture skips its
optional Caddy-adapt check when Caddy is absent, as it does on a runner without
Caddy. Wallet CSP checks download hash-pinned adapters and need network access.

## Cloud Checks after pushes

1. Sign in at <https://dagger.cloud> and select the intended organization.
2. Cloud Checks requires a Dagger Cloud **team account**. Review plan and billing
   terms before enrolling; this repo setup does not select a paid plan.
3. Under **Settings > Git Sources**, install the official Dagger Cloud GitHub
   App on **TraceCommons**, granting access to **only `trace-commons`**.
4. Commit/push `dagger.json` and `.dagger/` to the connected repository. Cloud
   discovers the annotated checks and runs them on new push events. A commit
   pushed before app installation needs another push to trigger checks.
5. Verify all six results on the Cloud **Checks** page and their GitHub commit
   statuses. Keep existing merge protection until equivalent coverage and
   merge-queue reporting have been verified.

No extra Actions runner or Dagger Cloud token is required for native Cloud
Checks. `dagger login <org>` connects local runs to that organization for Cloud
observability and the organization's configured runners/cache features. Verify
that runner/cache configuration in Cloud; logging in alone is not evidence of
cross-machine cache sharing. Never commit Cloud tokens or copy production
credentials into CI. These checks use synthetic local fixtures and no project
secrets.

## Faster feedback without extra compute or tokens

Dagger represents the pipeline as a graph and caches operations and function
results by their inputs. Unchanged successful work can be reused; a changed
input invalidates the affected work. Tools and browser downloads happen before
source is inserted, so code edits reuse those layers. A shared dependency-fetch
step completes before Rust lanes fan out. Cargo registry/Git caches retain
downloads, while separate build, Clippy, and test target caches avoid Cargo's
output-directory lock serializing the lanes. Each lane uses at most four Cargo
jobs; three compiling lanes can use twelve jobs in total.

Local containers use the engine's native architecture. Cargo cache identities
include platform, pinned Rust version, and lane. ARM laptop artifacts do not
serve as AMD64 Cloud artifacts. Cross-machine savings depend on compatible
platforms and the configured Cloud cache/runner; this local demo establishes
only local reuse. Cache eviction, changed tool versions, and network-dependent
inputs can reduce reuse. No fixed speedup or cost saving is promised.

For an agent, discover checks once, run a relevant lane while editing, then run
all portable checks before pushing. Successful cached work needs neither another
compile nor another agent debugging cycle. On failure, inspect the failing
operation and a bounded log excerpt instead of feeding the entire build log
back into a model. The runner executes code; the model is needed for diagnosis
and changes, not for repeatedly planning or narrating deterministic steps.
Tokens are saved by that workflow, not automatically by a compiler cache.

Do not cache time-sensitive security scans indefinitely. When migrating the
advisory workflow later, make its advisory-database revision or freshness policy
an explicit input. Do not retire existing jobs until their behavior is covered
and the replacement has been verified in Cloud.

## Evaluation on Google Compute Engine

The initial evaluation can run entirely on a dedicated Linux VM. Copy a frozen
source archive (tracked repo files plus this module, excluding credentials and
host artifacts), install Docker and the same Dagger version, and run
`scripts/ci/dagger-cache-demo.sh` there. Fetch `.local/dagger-runs/` afterward.
An 8-vCPU, 32-GB VM with a 150-GB boot disk has room for the three Rust lanes.

Use a dedicated evaluation VM with no attached service account, keep the Docker
socket local to that VM, and access it over SSH. Set a two-hour
`--max-run-duration` with `--instance-termination-action=DELETE` and auto-delete
its boot disk so an abandoned evaluation cannot run indefinitely. Fetch the
logs before termination. A fresh AMD64 server starts with a fresh Linux/AMD64
cache; the laptop's ARM cache is not part of its first-run timing. This VM
executes Dagger; GitHub push automation is still configured in Dagger Cloud.
