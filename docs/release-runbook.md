# Release Runbook

This document records the operational steps and invariants for cutting a
signed, distributable release of the Trace Commons contributor apps (the
macOS app bundle, the CLI, and the Linux flatpak). It is the companion to
`crates/trace-commons-contributor/tests/release_pipeline.rs`, which pins the
properties this runbook describes wherever they are visible in text (env
contracts, mandatory flags, deliberate exclusions). What the tests cannot
prove — that signing, notarization, or a flatpak build actually succeed
against real credentials — is covered by the manual gates below.

## Homebrew

The macOS app and CLI are distributed via a Homebrew tap:
`TraceCommons/homebrew-tap` (public, on GitHub), consumed as
`brew tap TraceCommons/tap`.

- `Casks/trace-commons.rb` installs the notarized `TraceCommons.app` from the
  DMG published as `TraceCommons-<version>.dmg` on releases of
  `TraceCommons/trace-commons-server` tagged `app-v<version>`.
- `Formula/trace-commons-contributor.rb` installs the signed CLI from the
  zips published as `trace-commons-contributor-<target>.zip` on releases
  tagged `contributor-v<version>`, for both `aarch64-apple-darwin` and
  `x86_64-apple-darwin`.
- Minimum macOS is Sonoma (`depends_on macos: ">= :sonoma"`), matching the
  app's `LSMinimumSystemVersion` of `14.0`.

### The app DMG is a universal binary

`macos/scripts/make-app-bundle.sh` builds the FFI dylib for both
`aarch64-apple-darwin` and `x86_64-apple-darwin`, `lipo`s them into one
dylib, and passes `swift build --arch arm64 --arch x86_64` so the app
executable is universal too; it verifies both with `lipo -archs` and fails
loudly if either is thin. `TraceCommons-<version>.dmg` therefore runs on both
Apple silicon and Intel Macs, and the cask carries no `depends_on arch:`.

The CLI (`Formula/trace-commons-contributor.rb`) ships separate
`aarch64-apple-darwin` and `x86_64-apple-darwin` builds instead of a
universal one; that is unrelated to the app DMG and unaffected by this.
- The cask's `uninstall quit:` stanza targets bundle identifier
  `ai.tracecommons.shell`, because the app registers itself as a login item
  via `SMAppService`; removing a running bundle without quitting it first
  strands an entry in System Settings > General > Login Items.

### The deliberate zap exclusion

The cask's `zap` stanza does **not** trash
`~/Library/Application Support/trace-commons/contributor.json`.

That file is the device identity key, and the server's `/v1/onboard` is
**not idempotent** — an invite code can be redeemed exactly once. If
`brew uninstall --zap` deleted `contributor.json`, a contributor who
reinstalls would have no way to re-enroll: their original invite code is
already spent and cannot be reissued. Omitting this path from `zap` is
intentional, and the cask carries a comment saying so, precisely because an
incomplete-looking `zap` stanza invites someone to "finish" it later. Don't.

### Checksum placeholders

No release has been published yet as of this writing, so the real
`sha256` values for the DMG and CLI zips are unknown. The cask and formula
currently carry obviously-invalid placeholders (e.g.
`sha256 "REPLACE_ON_FIRST_RELEASE_see_docs_release-runbook.md"`) rather than
a plausible-looking but wrong hex string — a wrong checksum makes Homebrew
report what looks like tampering, whereas an obviously-missing one just
looks unfinished.

**Action required on the first release:** compute the real `sha256` for each
published artifact (`shasum -a 256 <file>`) and replace the placeholders in
`Casks/trace-commons.rb` and `Formula/trace-commons-contributor.rb`, along
with the `version` fields. After the first release, Task 12's version-bump
automation takes over updating these values on subsequent releases.

## Publishing and the tap bump (Task 12)

Both `release-apps.yml` and `release-contributor.yml` end in a `publish`
job (gated to `github.event_name == 'push'` — a `workflow_dispatch` run
never publishes) that:

1. Downloads the platform artifacts and creates a GitHub Release with
   `gh release create`, tagged from the pushed tag.
2. Opens a pull request against `TraceCommons/homebrew-tap` bumping the
   cask's (or formula's) `version` and `sha256`, then **merges that pull
   request automatically**. The pull request is still opened rather than
   pushed directly, because it is the audit trail and the place the tap's
   own checks run — but nothing waits on a human.

**Nothing here is a manual step any more, and it must not become one
again.** Between 0.4.1 and 0.4.7, eleven bump pull requests sat unmerged
because this document stopped at "opens a pull request" and never told
anyone to merge them. The tap stayed on cask 0.4.0 and formula 0.3.0
while releases kept shipping, so every Homebrew user silently stopped
receiving them.

The verification gate people assumed the manual merge provided was never
the merge. It is upstream, on the job: in `release-apps.yml` the cask step
carries `if: needs.macos.result == 'success'`, and the macOS job builds,
signs, notarizes and staples the DMG before writing the checksum the step
reads; in `release-contributor.yml` the publish job `needs: build`. A
failed verification never reaches the bump step at all.

**The merge is still a manual step, deliberately.** Automating it is
wanted, but `TraceCommons/homebrew-tap` currently has `allow_auto_merge`
disabled, no branch protection on its default branch, and no CI workflows
at all. `gh pr merge --auto` would fail there outright, and falling back to
a plain `--squash` would be an immediate ungated merge into what
`brew upgrade` serves — the direct push this step exists to avoid, wearing
a pull request as a disguise.

Automating it needs something to gate on first: a cask/formula audit
workflow on the tap, auto-merge enabled on that repository, and that check
made required. Then, and only then, `gh pr merge --squash --auto` with no
fallbacks.

So: **after every release, merge the bump pull request.** The publish step
prints a `::notice::` and writes the pull request URL to the run summary
under "ready to merge". Homebrew serves the previous version until you do.

The weekly `tap-bump-staleness` workflow is the backstop: it fails if any
`bump-*` pull request on the tap has been open for more than a day. It is
deliberately not part of CI or of a release, so it can never fail a code
pull request or a release over a previous release's leftovers.

### Re-running a release tag

A deleted-and-re-cut tag re-runs these steps against branches that already
exist (`bump-cask-$V`, `bump-formula-$V`, `TraceCommons.Contributor-$V`).
That used to fail the publish job with `! [rejected] (fetch first)` after
every artifact had published. The steps now force-push.

Force-pushing is only safe because of how the branch is built: each step
clones the tap (or the winget fork) with `--depth 1 --single-branch`,
which fetches the *default* branch and nothing else, and never fetches,
checks out or reads the bump branch. The branch is reconstructed from the
default branch plus a checksum computed from this run's own artifact, so
there is no path by which a previous attempt's bytes reach the push. This
matters concretely: on the `app-v0.4.7` re-run the DMG was rebuilt, and
the branch left behind by the first run named a hash that no longer
matched the published asset. A force-push that carried that hash forward
would have shipped a broken `brew install --cask trace-commons` to every
macOS user.

Each step also asserts, after substitution and before pushing, that the
hash it computed is actually present in the file it is about to commit,
and exits 0 without pushing if the tap already carries that exact content.
Opening the pull request tolerates one already existing for the branch:
the step looks for an open pull request on that head first and reuses it,
since the force-push has already updated its contents.

This second step authenticates with the `HOMEBREW_TAP_TOKEN` repository
secret — a fine-grained PAT scoped to `TraceCommons/homebrew-tap` with
`contents` and `pull-request` write. `github.token` cannot reach another
repository, which is why this secret exists. Merging a pull request (and
enabling auto-merge on one) needs `pull-request` write and nothing more,
so the same secret covers the merge; the `tap-bump-staleness` workflow
reuses it read-only. It must be rotated like any
other credential in this pipeline; rotating it does not require touching
the workflow files, only the secret's value.

In `release-apps.yml`, the `publish` job carries `if: always() && ...` so
a Linux flatpak failure does not withhold a verified macOS DMG — but
`always()` alone would also let the job run when *every* platform failed,
with nothing to publish. The condition additionally requires
`needs.macos.result == 'success' || needs.windows.result == 'success' ||
needs.linux-flatpak.result == 'success'` to close that gap, and the cask
bump step only runs when `needs.macos.result == 'success'` (there is no
DMG checksum to read otherwise).

The formula's `sha256` substitution is positional, not value-based: the
Homebrew formula's `on_arm` block appears before `on_intel`, so a blind
global substitution would set both entries to the same hash. The bump
step uses `awk` to replace the first `sha256 "..."` occurrence with the
`aarch64-apple-darwin` checksum and the second with `x86_64-apple-darwin`
— getting this backwards produces a checksum mismatch that reads to users
as tampering, not as a build error. This works whether the existing value
is the `REPLACE_ON_FIRST_RELEASE_...` placeholder or a real hash from a
prior release.

### What to expect to fail on the first real tagged release

This automation has never run against real credentials or a real tag.
Expect friction in roughly this order:

- **`HOMEBREW_TAP_TOKEN` scope or expiry.** Fine-grained PATs expire; if
  this one lapsed since Task 11 provisioned it, `gh repo clone` or
  `gh pr create` in the bump steps will fail with an auth error, not a
  silent no-op.
- **`gh repo clone ... -- --depth 1` and `git push -u origin`.** Never
  exercised end-to-end. If the PAT's `contents` scope is read-only instead
  of read-write, the `git push` step fails after the commit succeeds
  locally — leaving a half-done local branch in the ephemeral runner
  workspace, which is harmless, but worth recognizing in the log rather
  than assuming a truncated run means nothing happened.
- **A second `gh pr create` on a re-run.** Both bump steps use a
  version-derived branch name (`bump-cask-$V`, `bump-formula-$V`) with no
  cleanup of a previous attempt's branch. Re-running the same tag (a retry
  after a transient failure) will fail at `git switch -c` or `gh pr create`
  because the branch/PR already exists. There is no idempotent retry path
  yet — a failed bump currently needs a human to delete the stale branch on
  the tap before re-running.
- **Empty `dist/*/*` glob.** If every platform legitimately failed (not
  possible on the first real run without the `always()` guard already
  refusing to publish, but worth confirming): the guard added in this task
  should prevent `gh release create` from running with nothing to attach.
  Confirm this in practice, not just by reading the condition.
- **The macOS checksum sidecar path.** `dist/macos-dmg/TraceCommons-$V.dmg.sha256`
  assumes the exact filename the macOS job produces. Any future rename of
  that job's output (`Rename and checksum` step) silently breaks this glob
  with no test coverage catching it — `release_pipeline.rs` checks text
  invariants, not that the two jobs agree on a filename.
- **The formula's checksum sidecar paths**, same risk, doubled: both
  `aarch64-apple-darwin` and `x86_64-apple-darwin` artifact names must stay
  in lockstep with the matrix in `release-contributor.yml`.
- **Task 11's Step 8 install gate** (`brew tap` / `brew install --cask` /
  `brew install` against the real, merged tap) has not been run against a
  real release yet and still needs to happen once the first tap PR merges.
