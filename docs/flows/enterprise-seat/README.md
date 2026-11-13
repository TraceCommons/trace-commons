# Enterprise seat flow

A Quint model of one member's Mac and one server seat through the life of an org, for the
design in `docs/superpowers/specs/2026-11-11-enterprise-gateway-design.md`. Made with the
flow-promises skill: mocks of each state, a model of how the member and the world move between
them, and promises in the product's words, checked by TLC.

- `enterprise_seat.qnt`: the model and the promises.
- `enterprise_seat.cfg`: the TLC config, one line per promise.
- `view.mts`, `labels.mts`: the check's view (chart labels and step labels).
- `render.mts`: the render-only view for the merged chart (hides steps; see below).
- `enterprise_seat.mmd`, `enterprise_seat.svg`: the merged chart.
- `mocks/index.html`: the design surface, with the chart beside the frames.
- `chart.sh`, `merge.mts`: redraw the merged chart.

## Promises

Checked by TLC on all 1,217,568 states of the model. All eleven hold, and no state is a dead end. A probe that negates promise 11 under the same assumptions breaks, so its assumptions are satisfiable and the promise is not vacuous.

| # | Promise | Result |
|---|---|---|
| 1 | Joining never carries your Automatic over: when you join, every folder is back on Ask. | Holds |
| 2 | Your traces leave only by your act: every trace your Mac sends to the org was submitted by you, or sent by an Automatic you turned on after joining. | Holds |
| 3 | If the org forbids Automatic, nothing of yours goes automatically. | Holds after design change A |
| 4 | Sharing off means no new credit: while sharing is off, nothing from you or a server is credited. | Holds |
| 5 | Removal stops uploads: after the admin removes you, nothing more from your Mac is credited, including an upload already in flight. | Holds |
| 6 | Leaving the NEAR AI org stops uploads by your next sign-in. | Holds |
| 7 | You can take back what you gave the org: a trace credited to the org can be withdrawn while you are signed in or during the removal window. | Holds |
| 8 | What you earned personally stays withdrawable. | Holds after design change B |
| 9 | Servers share only under the org's consent: only after the org accepted the server terms, and only while the server is enrolled. | Holds |
| 10 | The org closes cleanly: it closes only after its credit settles, and a closed org credits nothing new. | Holds |
| 11 | A submitted trace eventually lands: if you stay a member, keep signing in, sharing stays on, the network works and the org stops changing policy, a trace you submit is eventually credited. | Holds after design change C and two owner rewrites |

## Design changes the counterexamples caused

Each one is the owner's pick, 2026-11-11, from the fixes offered against the path.

**A. The upload claim carries the policy (promise 3).** Path: you join; you choose Automatic; the
agent writes a trace; the admin forbids Automatic; Automatic sends it, because the Mac enforced
Automatic from the policy it last fetched. Change: an org claim carries the policy digest the Mac
acted on and whether Automatic sent it. The issuer refuses a stale digest, and an Automatic claim
the org forbids; the trace goes back to Held and nothing leaves the Mac. Cost: any policy change
refuses the claims in flight once. Spec impact: section 4.4 (`member_automatic` is no longer
client-only) and the `consent_basis` claim field in 5.3.

**B. "Before you joined" keeps Withdraw for personal credit (promise 8).** Path: a personal
Automatic upload is in flight; you press Join; it is credited to your personal account while you
join; the managed app has no Withdraw for it. The same holds for any trace credited before joining.
Change: the Mac keeps a withdraw-only credential for the personal account, and a "Before you
joined" list with Withdraw on every screen. Cost: a narrow exception to one enrolment per Mac
(spec decision 4).

**C. Uploads wait while you are signed out (promise 11).** Path: your session keeps expiring just
before Automatic sends, so every upload leaves while you are signed out and the issuer refuses it.
Change: in an org, the Mac neither submits nor sends automatically while its org session has
expired; traces stay Held until you sign in again.

**Promise 11 rewritten by the owner, twice.**

- After A to C it still broke: an admin who toggles "forbid Automatic" forever makes each claim
  stale, and the issuer refuses each one, which is change A working. The owner added "and the
  org stops changing policy".
- It then broke because a session that expires during every upload gets each one refused on
  arrival: ingest re-checks membership freshness when the upload lands, which keeps promise 6
  strict. The owner kept that check and added "and your sign-in eventually lasts long enough"
  (org sessions are 12 hours; an upload takes seconds).

## Model fix (not a design change)

The first model had one refusal for a claim, with no fairness, so a claim the issuer could not
grant could wait forever. A real issuer always answers. The model now separates "the issuer
refuses the claim" (only when it cannot grant it, and fair under "the network works") from "the
claim request fails" (network, any time).

## What the model assumes and leaves out

- Spec recommendations taken as settled: NEAR AI organization membership is the org's identity
  (decision 1); credit goes to the org only (decision 3); one enrolment per Mac, except change B
  (decision 4); removal opens a withdraw-only window (decision 6), whose length is not modelled.
- Org sessions do not slide; an expired one is renewed only by a fresh membership check (spec 5.3).
- One member, one server, one trace at a time for each. Several members, several servers and
  several traces in flight are not modelled.
- The "Join your organization" control stands for following an org's enrolment link or a
  configuration profile (spec 9.1); there is no in-app join for a person who has neither.
- Data uses, the privacy scan, scoring, dedup, payout and the admin console's own sign-in are
  outside the model.
- The rule-9 surfaces are placeholders in the mocks; their wording waits on spec decision 9.
- Behind "holds": promise 11 also assumes you submit again after each refusal and that the Mac
  keeps fetching policy (fairness on those steps). Promises 2 and 3 assume an honest client: how a
  trace was sent is asserted by the Mac, and the server cannot tell a member's act from a policy
  (spec 3.1).
- Where checks run: the issuer checks the policy digest and Automatic; the upload's arrival
  re-checks org status, sharing, the seat, the session (membership freshness) and, for the
  server, the server terms. Promises 4, 5, 6, 9 and 10 depend on those arrival checks (spec 5.3).

## The chart

The model is far above the checker's 500-state chart budget, so the chart is merged for drawing
only, as the skill's step 5 describes: TLC dumps every state and step, and `render.mts` labels
them. Every model state and step stays in; the chart draws fewer of them.

- Merged: the member's Mac by screen (and session for Managed); the latest trace by its stage and
  holder; the server by seat; the org by status (and sharing while Active).
- Hidden steps (still in the model and checked): choosing Ask or Automatic, the Mac fetching
  policy, NEAR AI removing or re-adding you, the admin forbidding or allowing Automatic or
  accepting server terms, settlement, the server's own uploads, and the claim request failing.

## Check it again

Needs Java 21+, Quint 0.33.0, and `tla2tools.jar` 1.7.4 (sha256
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`).

Check a copy. When every promise holds, `scripts/check` draws no chart for a model this size,
and it then deletes `enterprise_seat.mmd` and `enterprise_seat.svg` beside the flow it checked.

```sh
export TLA2TOOLS=/path/to/tla2tools.jar JAVA=/path/to/java
tmp=$(mktemp -d) && cp -R . "$tmp" && cd "$tmp"
<flow-promises skill>/scripts/check enterprise_seat.qnt --view view.mts --out out
```

To redraw the merged chart: `FLOW_PROMISES=<skill dir> TLA2TOOLS=... JAVA=... ./chart.sh`. It
dumps the full graph with TLC (`-dump dot,actionlabels`, no promises in the config) and draws it
with the checker's own passes (`merge.mts`) and the render view (`render.mts`).
