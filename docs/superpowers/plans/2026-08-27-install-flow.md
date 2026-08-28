# Implementation plan: shorten the contributor CLI installation flow

Date: 2026-08-27 (revised after recovering Paxel's actual flow)
Feedback: Devfolio item 2 — "Can we simplify the CLI process to a one-step
command? Check how Paxel is doing it." Companion spec
`docs/superpowers/specs/2026-08-27-hackathon-onboarding-friction-design.md`
lists installation as out of scope for Slices A-F; this plan fills that gap.
Paths are relative to the worktree root (branch `devfolio-feedback-3`).

Hard constraint honored throughout: the verification posture does not move.
install.sh refuses anything it cannot verify (checksum always; Developer ID
naming team KXSWJN7WY8 on macOS), has no `--force` and no `--skip-verify`
(install.sh:14-27), and nothing here adds one.

## 1. What Paxel actually does, and what it changes

paxel.ycombinator.com is geo-blocked (confirmed from a real browser session),
but the steps were recovered from Devfolio's own feedback page (see
`../paxel-actual-flow.md`):

- All repos: `curl -fsSL https://paxel.ycombinator.com/upload.sh | bash`,
  run from the parent folder holding the repos.
- One repo: the same command, run from that project's folder.

Four consequences, three of which change this plan's premise:

1. **There is no installation.** The praised "installation flow" is
   `curl | bash` executed directly — no binary on PATH, no persistent state,
   no second command. The thing Devfolio is pointing at maps to the spec's
   Slice F one-time script, not to install.sh.
2. **It is one command, not two.** Their two "commands" differ only in
   working directory.
3. **Scoping is by directory subtree, not by time.** Our bare `submit`
   scopes by a 7-day window across all projects — a different selection
   entirely. That is Slice C/F territory, not installation, but the Slice F
   plan owner needs it: the one command's *meaning* is part of what was
   praised.
4. **Paxel does no verification at all** — an unsigned script piped to bash.
   This is the one place we do not follow them, and the asymmetry is
   defensible: Paxel leaves nothing persistent behind; we install a binary
   that reads coding transcripts. The constraint stands unchanged.

### The honest conclusion

Most of the installation work in the first draft of this plan is now lower
priority than making Slice F a single piped command. The bar is not "install
in fewer steps"; it is "nothing the hacker experiences as installation."
install.sh still matters — Slice F reuses `install.sh --dir` into a cache
directory, so install.sh becomes the load-bearing interior of the praised
flow — but its *user-facing* ergonomics (PATH persistence, Homebrew command
count, winget) serve the durable-install audience, not the hackathon one.
The plan below is reordered accordingly: P0 tasks serve the Slice F flow,
P1 tasks fix real defects regardless of audience, P2 tasks are durable-install
polish that should not gate anything.

## 2. What installation costs today (unchanged facts)

| Path | Commands | Hidden extra steps | Honest total |
| --- | --- | --- | --- |
| macOS/Linux, curl | 2 (`curl -o`, `sh install.sh`) | On default macOS zsh `~/.local/bin` is not on `PATH`; install.sh:162-169 prints a non-persistent `export` suggestion only, so the user edits an rc file or loses the fix. First symptom: `command not found` after a "successful" install. | 3-4 actions |
| macOS, Homebrew | 3 (`brew tap`, `brew trust`, `brew install`) | `brew trust` is obscure. | 3 |
| Windows, PowerShell | 2 (`irm -OutFile`, `.\install.ps1`) | Appends user PATH itself (install.ps1:239-246); terminal reopen required. | 2 + reopen |
| Windows, winget | not usable | Unpublished **by design**: release-contributor.yml:637 deliberately does not open the winget-pkgs pull request — the token cannot, and doing so failed the contributor-v0.4.7 release — so each release force-pushes a fork branch and prints a compare URL (:690) for a human. The Homebrew tap bump, by contrast, auto-opens its PR (release-contributor.yml:597). Whether any winget PR was ever opened cannot be established from this repo; what is established is that the step is manual and winget serves nothing today. | n/a |

Two repo defects independent of audience:

- **scripts/install.sh has zero CI coverage.** `grep -r install.sh
  .github/workflows/` returns nothing; install.ps1 has a full job
  (`windows-install-script`, ci.yml:448-531) that installs a pinned real
  release and asserts refusal-with-nothing-installed. The sh script that
  every macOS/Linux user and now Slice F depends on is tested by nobody.
- **install.sh executes linearly under a pipe.** install.sh:8 permits the
  piped form; a truncated download executes a prefix. Harmless today
  (nothing destructive precedes verification), unacceptable the moment a
  piped one-liner is the advertised flow — which Slice F makes it.

## 3. Tasks in dependency order

### P0 — serves the Slice F flow directly

**Task 1 — `main`-wrap install.sh.**
File: `scripts/install.sh`. Move the linear body into `main()`; last line
`main "$@"`. No behavior change. The same pattern is a requirement on the
Slice F script itself (flag to its plan owner): both will be fetched over
the network and executed, and Slice F's is the one that gets piped by
design. Watch the `--help` implementation — it is a `sed` range over the
file's own header (install.sh:45) and both this task and any header growth
move it.
Test: `sh scripts/install.sh --help` prints the header;
`head -c 500 scripts/install.sh | sh` executes nothing.

**Task 2 — CI job for install.sh.**
File: `.github/workflows/ci.yml`. Add `install-script` on `ubuntu-24.04`
and `macos-26`, mirroring `windows-install-script` (ci.yml:448-531):
- pinned `--version` install into a temp `--dir` under a temp `HOME`
  (pinning avoids the tag-resolution API call and keeps the job
  deterministic, exactly as the windows job argues at ci.yml:482-488);
  assert the binary runs `--version`;
- tamper the expected checksum the way the windows job rewrites its signer
  constant (ci.yml:503-510); assert non-zero exit and zero files installed;
- macOS leg: rewrite `EXPECTED_AUTHORITY` and assert refusal on a wrong
  signing identity;
- assert `--dir` never touches PATH or rc files — this is the exact
  contract Slice F depends on.
Honor the pinned-by-SHA action versions already in ci.yml.
Test: the job is the test.

**Task 3 — hand the scoping finding to the Slice F plan.**
No file here. Paxel scopes by directory subtree; our bare `submit` scopes
by a 7-day window across all projects. A hacker running the one-time script
from `~/code` expects "the repos under here" and gets "everything in 7
days". Whether Slice F's script should `cd`-scope (and how that composes
with Slice C's one-step submit) is that plan's decision, but it must be
made deliberately, not inherited silently. Also relay: Paxel reads Cursor
transcripts — relevant to feedback item 1's adapter list, not to this plan.

**Task 4 — where the one-liner lives (decision needed, then small).**
The Slice F command must be one short pasteable line. install.sh:10-12
records the decision against a prettier URL for install.sh; Paxel's URL is
short because it is first-party hosted. Options, in order of preference:
(a) serve the Slice F script at `https://tracecommons.ai/hackathon.sh` as
a 301 to the raw GitHub URL — never a second copy; drift is worse than a
long URL — with a weekly end-to-end curl in a scheduled workflow (pattern:
`.github/workflows/tap-bump-staleness.yml`) and the raw URL documented as
fallback; or (b) accept the raw.githubusercontent URL in hackathon
materials and skip the redirect. Either is workable; (a) needs whoever owns
the site, which this repo does not. install.sh's own URL stays as is —
the recorded decision is only worth reopening for the command hackers
actually paste.

### P1 — real fixes for the durable install, not gating Slice F

**Task 5 — PATH persistence in install.sh.**
File: `scripts/install.sh`. After the existing check (install.sh:162-169):
unless `--no-modify-path`, and only when `$SHELL` basename is zsh or bash,
append a marker-guarded `export PATH` block to `~/.zshrc`/`~/.bashrc`,
idempotently; other shells keep the printed instruction, now naming the rc
file. A failed append degrades to the printed instruction, never fails the
install, and never touches any other file. Mirrors install.ps1's existing
behavior (install.ps1:239-246). Document the flag in the header/`--help`.
Slice F is unaffected by construction: it passes `--dir` into a cache
directory, and Task 2's CI asserts `--dir` runs make no PATH or rc changes.
Test: extend Task 2's job — fresh HOME gains exactly one block; second run
adds nothing; `--no-modify-path` leaves the rc untouched.

**Task 6 — README.**
File: `README.md` (:178-213). Keep the two-step download-then-read form
first (install.sh:4-8 is deliberate and stays); add the piped one-liner
immediately after for people who want it — verification is identical in
both forms, and Task 1 has made the pipe truncation-safe; document
`--no-modify-path`. Add the Slice F one-liner to hackathon-facing material
once that slice ships (out-of-repo: tracecommons.ai/install — flag to the
site owner). Never document winget or a Homebrew short form before Tasks
7-8 establish they work.

### P2 — durable-install polish; do not let these crowd out P0

**Task 7 — Homebrew to one paste.**
Verify on a real mac whether `brew install
TraceCommons/tap/trace-commons-contributor` auto-taps with an interactive
trust prompt or hard-stops on the untrusted tap; paste the transcript in
the PR. If it prompts: README collapses to one command. If not: document
the `&&`-joined three commands as one paste. `brew trust` is Homebrew's
supply-chain control and is not removed. Update `docs/release-runbook.md`
(:12-16), and note this partially discharges the runbook's open Task 11
Step 8 install gate (:214-216).

**Task 8 — publish winget (process).**
Open the pull request the automation deliberately leaves to a human: take
the compare URL from the latest contributor release's job summary
(release-contributor.yml:688-690) and shepherd it through winget-pkgs
review (first-time package review can take weeks; external). Add a winget
section to `docs/release-runbook.md`: the fork-branch automation, the one
manual click per release, and the monitoring gap — `tap-bump-staleness`
watches only the Homebrew tap, so a never-opened winget PR raises no
alarm; extend it or add a sibling check for lingering
`TraceCommons.Contributor-*` fork branches. After merge, verify
`winget install TraceCommons.Contributor` on a clean Windows VM and record
the transcript. README mention lands only then (Task 6's rule).

## 4. What could go wrong

- **The biggest risk is building the wrong thing:** polishing install.sh
  ergonomics while the praised experience is Slice F. The P0/P1/P2 split
  exists to prevent that; if capacity forces a cut, cut from the bottom.
- **rc-file editing (Task 5)** can break a shell startup. Mitigations:
  zsh/bash only, one marker block, CI idempotence assertions,
  `--no-modify-path`, degrade-to-printed-instruction on any write failure.
- **`--help` regression**: the help text is a `sed` line-range over the
  header (install.sh:45); Tasks 1 and 5 both move it. Task 2's job asserts
  `--help` output contains the flag names.
- **Redirect outage (Task 4a)** makes the hackathon command look broken at
  the worst moment. The scheduled end-to-end curl plus a documented raw-URL
  fallback bounds it; if the site owner cannot commit to the redirect,
  choose 4b.
- **winget-pkgs review latency** is external and unbounded for a first
  package; nothing else depends on it, which is why it is P2.
- **Homebrew trust behavior varies by brew version** — hence Task 7
  verifies on a real machine before any README change and records the
  version tested.
- **Verification posture**: untouched by every task above. Any review
  suggestion of a `--skip-verify`, a checksum bypass, or Paxel-style
  unverified execution of the *binary* is out of bounds by design
  (install.sh:14-27). The Slice F script itself arriving over TLS and then
  running the fully verifying install.sh is the intended split: the script
  is same-origin-trusted either way; the binary never is.
