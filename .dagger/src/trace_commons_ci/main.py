"""Portable Trace Commons checks, shared by local runs and Dagger Cloud."""

from typing import Annotated

import dagger
from dagger import DefaultPath, Ignore, check, dag, function, object_type

RUST_IMAGE = "rust:1.99.1-bookworm"
NODE_IMAGE = "node:24.22.1-bookworm-slim"


@object_type
class TraceCommonsCi:
    source: Annotated[
        dagger.Directory,
        DefaultPath("/"),
        Ignore([
            ".git", ".dagger", "**/target", "**/node_modules", "**/.venv",
            ".local", ".build", ".cloudflare", ".claude", ".codex", ".agents",
            ".aws", "**/__pycache__", "**/*.pyc", "**/.env", "**/.env.local",
            "**/.env.*", "**/*.bak", "**/*.bak-*", ":memory:.*",
        ]),
    ]

    def _base(self) -> dagger.Container:
        # Install tools before adding source: editing code reuses this layer.
        node = dag.container().from_(NODE_IMAGE)
        return (
            dag.container().from_(RUST_IMAGE)
            .with_exec(["rustup", "component", "add", "rustfmt", "clippy"])
            .with_exec([
                "sh", "-ec",
                "apt-get update && apt-get install -y --no-install-recommends "
                "python3 gettext-base jq gnome-keyring dbus-x11 libdbus-1-dev "
                "pkg-config libssl-dev ca-certificates curl git perl zstd openssl "
                "&& rm -rf /var/lib/apt/lists/*",
            ])
            .with_exec(["useradd", "--create-home", "--uid", "1000", "ci"])
            .with_directory("/usr/local", node.directory("/usr/local"))
            .with_env_variable("RUSTFLAGS", "-D warnings")
            .with_env_variable("CARGO_INCREMENTAL", "0")
            .with_env_variable("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
            .with_env_variable("CARGO_BUILD_JOBS", "4")
            .with_env_variable("CARGO_PROFILE_DEV_DEBUG", "0")
            .with_env_variable("CARGO_PROFILE_TEST_DEBUG", "0")
            .with_env_variable("CARGO_TERM_COLOR", "never")
            .with_workdir("/src")
        )

    def _source(self) -> dagger.Container:
        return self._base().with_directory("/src", self.source, owner="1000:1000")

    def _as_ci(self, container: dagger.Container) -> dagger.Container:
        # Permission-refusal regressions must run without root's DAC bypass.
        return (
            container.with_user("ci")
            .with_env_variable("HOME", "/home/ci")
            .with_exec(["sh", "-ec", 'test "$(id -u)" -ne 0'])
        )

    def _test_tools(self, container: dagger.Container) -> dagger.Container:
        # Browser downloads are toolchain inputs, independent of project edits.
        return (
            container
            .with_env_variable("PLAYWRIGHT_BROWSERS_PATH", "/opt/playwright-browsers")
            .with_exec(["npm", "install", "--prefix", "/opt/wallet-ci", "--ignore-scripts", "--omit=optional", "--no-audit", "--no-fund", "--save-exact", "playwright@1.63.0", "playwright-core@1.63.0"])
            .with_exec(["node", "/opt/wallet-ci/node_modules/playwright/cli.js", "install", "--with-deps", "chromium"])
            .with_env_variable("TC_PLAYWRIGHT_MODULE", "/opt/wallet-ci/node_modules/playwright")
        )

    async def _rust(self, lane: str) -> dagger.Container:
        platform = str(await dag.default_platform()).replace("/", "-")
        prefix = f"trace-commons-rust-1.99.1-{platform}"
        # All Rust lanes depend on this identical fetch vertex. Fetch completes
        # before they fan out, avoiding concurrent writes to Cargo's Git DB.
        dependencies = (
            self._source()
            .with_mounted_cache("/usr/local/cargo/registry", dag.cache_volume(f"{prefix}-registry"))
            .with_mounted_cache("/usr/local/cargo/git", dag.cache_volume(f"{prefix}-git"))
            .with_exec(["chown", "-R", "1000:1000", "/usr/local/cargo"])
        )
        dependencies = self._as_ci(dependencies).with_exec(["cargo", "fetch", "--locked"])
        # Sync the common fetch, then reuse its downloaded volumes in every lane.
        await dependencies.sync()
        container = self._base()
        if lane == "tests":
            container = self._test_tools(container)
        container = (
            container.with_directory("/src", self.source, owner="1000:1000")
            .with_mounted_cache("/usr/local/cargo/registry", dag.cache_volume(f"{prefix}-registry"))
            .with_mounted_cache("/usr/local/cargo/git", dag.cache_volume(f"{prefix}-git"))
            .with_mounted_cache("/src/target", dag.cache_volume(f"{prefix}-{lane}-target"))
            # Existing evaluation volumes were populated as root. Keep paths
            # stable while making both existing and new cache entries writable.
            .with_exec(["chown", "-R", "1000:1000", "/usr/local/cargo", "/src/target"])
        )
        return self._as_ci(container)

    @function
    @check
    async def format(self) -> None:
        """Check formatting of every crate in the root Rust workspace."""
        await self._as_ci(self._source()).with_exec(["cargo", "fmt", "--all", "--", "--check"]).sync()

    @function
    @check
    async def safety(self) -> None:
        """Run the release, staging, privacy, and qualification guards from CI."""
        container = self._as_ci(self._source())
        for script, args in [
            ("check-release-credential-store.sh", ["check"]),
            ("check-release-credential-store.sh", ["self-test"]),
            ("check-release-staging-feature.sh", ["check"]),
            ("check-release-staging-feature.sh", ["self-test"]),
            ("test-macos-channel-plist.sh", []),
            ("assert-no-staging-markers.sh", ["self-test"]),
            ("test-generate-appcast.sh", []),
            ("test-deploy-render.sh", []),
            ("test-staging-db-backup.sh", []),
        ]:
            container = container.with_exec(["bash", f"scripts/ci/{script}", *args])
        for command in [
            ["python3", "-m", "unittest", "discover", "-s", "scripts/operator/tests", "-p", "test_anonymize_network_recording.py"],
            ["python3", "scripts/qualification/check-opencode-qualification.py"],
            ["python3", "scripts/qualification/test-opencode-qualification.py"],
        ]:
            container = container.with_exec(command)
        await container.sync()

    @function
    @check
    async def browser(self) -> None:
        """Run wallet/step-up state tests and the AASA HTTP contract tests."""
        container = self._as_ci(self._source())
        for command in [
            ["node", "scripts/ci/test-near-wallet-browser.cjs"],
            ["node", "scripts/ci/test-step-up-page.cjs"],
            ["node", "--test", "scripts/ci/test-check-aasa.mjs"],
        ]:
            container = container.with_exec(command)
        await container.sync()

    @function
    @check
    async def build(self) -> None:
        """Check the server/ingest binaries and contributor, including staging."""
        container = await self._rust("build")
        for args in [
            ["-p", "trace-commons-server", "-p", "trace-commons-ingest", "--bins"],
            ["-p", "trace-commons-contributor"],
            ["-p", "trace-commons-contributor", "-p", "trace-commons-contributor-ffi", "--features", "trace-commons-contributor/near-ai-staging"],
            ["-p", "trace-commons-server", "-p", "trace-commons-ingest", "--bins", "--features", "trace-commons-ingest/near-ai-staging"],
        ]:
            container = container.with_exec(["cargo", "check", "--locked", *args])
        await container.sync()

    @function
    @check
    async def clippy(self) -> None:
        """Lint workspace and staging variants with CI's unchanged allow list."""
        container = await self._rust("clippy")
        allowances = [
            "-D", "warnings", "-A", "clippy::type_complexity",
            "-A", "clippy::collapsible_if", "-A", "clippy::manual_option_as_slice",
            "-A", "clippy::useless_vec", "-A", "clippy::redundant_pattern_matching",
        ]
        for args in [
            ["--workspace", "--all-targets"],
            ["-p", "trace-commons-contributor", "-p", "trace-commons-contributor-ffi", "--all-targets", "--features", "trace-commons-contributor/near-ai-staging"],
            ["-p", "trace-commons-server", "-p", "trace-commons-ingest", "--all-targets", "--features", "trace-commons-ingest/near-ai-staging"],
            ["-p", "trace-commons-attestation", "--all-targets", "--features", "near-ai-staging"],
        ]:
            container = container.with_exec(["cargo", "clippy", "--locked", *args, "--", *allowances])
        await container.sync()

    @function
    @check
    async def tests(self) -> None:
        """Run workspace tests, real Secret Service round trips, and wallet CSP."""
        container = await self._rust("tests")
        for command in [
            "cargo test --locked --workspace",
            "cargo test --locked -p trace-commons-contributor --features near-ai-staging --test channel_hosts --test credential_store_isolation --test preflight --test version_channel",
            "cargo test --locked -p trace-commons-contributor --features near-ai-staging --lib -- other_channel the_config_dir_help the_default_inbox private_inference config::tests:: deep_link invite_tests ironwire_pointer discover ironwire_folder a_staging_probe_of a_staging_channel_shows_the_label proof_attestor pin_refusal reconcile_never_fetches_the_signed_pins",
            "cargo test --locked -p trace-commons-contributor-ffi --features trace-commons-contributor/near-ai-staging --test abi --test build_channel",
            "cargo test --locked -p trace-commons-attestation --features near-ai-staging",
            "cargo test --locked -p trace-commons-server --features near-ai-staging --bin trace-commons-staging-pins --test staging_pins_tool",
            "cargo test --locked -p trace-commons-contributor-platform --lib real_os_round_trip_removes_only_its_synthetic_entry -- --ignored",
            "cargo test --locked -p trace-commons-contributor-platform --lib real_commons_os_round_trip -- --ignored",
            "node scripts/ci/test-near-wallet-sdk.cjs",
            "cargo test --locked -p trace-commons-contributor --lib near_wallet_page::tests::pinned_wallet_controls_obey_production_csp -- --ignored --test-threads=1",
        ]:
            container = container.with_exec([
                "dbus-run-session", "--", "sh", "-ec",
                'printf "\n" | gnome-keyring-daemon --unlock --components=secrets; ' + command,
            ])
        await container.sync()
