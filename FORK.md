# rdisandro/trace-commons

A fork of [TraceCommons/trace-commons](https://github.com/TraceCommons/trace-commons)
for iterating on the client UX, kept at feature parity with upstream `main`.
Everything upstream ships is here; this fork adds front-end work on top, and
every piece of that work is offered back upstream as its own pull request.

## Branches

| Branch | What it holds | Offered upstream? |
| --- | --- | --- |
| `main` | Upstream `main`, plus merged UX work, plus the fork-only files below | Never as a whole |
| `ux/<topic>` | One change, cut from **upstream** `main` | Yes, as a PR from this branch |

A `ux/` branch starts from upstream `main`, not from this fork's `main`, so a
PR from it carries only that change: none of the fork-only files, and none of
the fork's other unmerged work. When a change needs another one that upstream
has not merged yet, cut it from that branch and say so in the PR.

Fork-only files, never in a `ux/` branch:

- `.github/CODEOWNERS`
- `FORK.md`

Merged here and still open upstream: #2005, which includes #1997.

## Keeping parity

Upstream lands about 70 commits a day. Merge it into this fork's `main` at
least once a day, so conflicts stay small. Merge, never rebase: `main` is
shared.

```bash
git fetch upstream
git switch main
git merge upstream/main
git push origin main
```

On a conflict in a fork-only file, keep this fork's version.

Starting a change:

```bash
git fetch upstream
git switch -c ux/<topic> upstream/main
```

When it is ready, merge it into this fork's `main` and open the upstream PR
from the same branch:

```bash
gh pr create --repo TraceCommons/trace-commons --base main --head rdisandro:ux/<topic>
```

The remote names above assume a clone of this fork (`origin` is the fork,
`upstream` is TraceCommons). In a clone of TraceCommons they are the other way
round.

## For upstream code owners

To take a change from this fork:

- **Review its PR.** Each change arrives as a PR from `rdisandro:ux/<topic>`
  against `TraceCommons:main`, so it goes through upstream's CODEOWNERS, branch
  protection, CI and merge queue as usual. Upstream's CI needs no secrets for a
  PR. Maintainer edits are allowed, so you can push fixes to the branch.
- **Or cherry-pick it yourself:**

  ```bash
  git fetch https://github.com/rdisandro/trace-commons.git ux/<topic>
  git cherry-pick <commits>
  ```

- **See everything the fork has that upstream does not:**
  [TraceCommons:main...rdisandro:main](https://github.com/TraceCommons/trace-commons/compare/main...rdisandro:trace-commons:main)

## What runs here

CI (`ci.yml`), `cargo-deny`, the client suites (`clients.yml`) and the
OpenCode qualification run on every push to `main` and every PR, on standard
GitHub runners.

Disabled, because they need TraceCommons signing, notarization, Sparkle and
publishing secrets that a fork does not have: `release-apps`,
`release-contributor`, `staging-macos`, `tap-bump-staleness` and
`witness-image`. This fork makes no signed builds, releases or staging
publishes. Build the apps locally instead.

## Licensing

Unchanged from upstream: the split license in `LICENSE` and `AGENTS.md`
applies here as is, and changes offered upstream are licensed inbound under
`MIT OR Apache-2.0`, per `CONTRIBUTING.md`.
