# Sizzle video brief — the Trace Commons contributor apps

Audience for this document: a designer or motion designer producing a short
(60–90s) sizzle video that explains and demos the desktop contributor apps to
people who have never seen them.

Everything below is drawn from the shipped shells and the specs behind them
(`docs/superpowers/specs/2026-08-08-contributor-shell-*-design.md`). On-screen
copy quoted here is the copy in the product — use it verbatim rather than
paraphrasing, because the exact wording is load-bearing (see "The premise").

---

## 1. What the apps are, in one sentence

Three native desktop apps — macOS, Windows, and Linux — that watch for finished
Claude Code and Codex sessions on your machine, scrub them locally, show you
exactly what would be sent, and send nothing unless you say so.

Longer, from the app's own first screen:

> Coding agents get better when there are real transcripts to learn from.
> Almost all of that data is locked inside companies. Trace Commons is a
> shared pool that isn't.

## 2. The premise the video has to survive

The app asks a developer to upload transcripts of their real work — possibly
their employer's or a client's — automatically, in the background, for credit
that is explicitly not money. No amount of reassuring tone makes that
acceptable. The interface has to be demonstrably more careful than the user is,
and careful in ways they can *see*.

Four properties carry that weight. **The video's job is to show all four.**

1. **The first thing the app ever does is nothing.** Onboarding ends on the
   words "You're set up. Nothing has been sent."
2. **The user watches the system refuse.** When a session changes between
   approval and upload, the daemon declines to send it — loudly, not silently.
3. **"Not sent" is always distinguishable from "sent."** Pending, refused,
   expired and superseded all mean nothing left the machine, and the queue says
   so in words.
4. **The app can always explain itself.** Why a session wasn't offered, why
   something is waiting, what a status means.

### Things the video must never imply

These are documented uninstall triggers. Do not stage a shot that suggests any
of them:

- An upload action reachable from a notification. There is none, ever.
  Notification actions are **Review** and **Not now**.
- Anything being uploaded that the user did not decide on.
- A trace stuck in review forever with no explanation and no way to take it
  back.
- Being asked for credentials for a "privacy filter" the user never asked for.

## 3. Cross-platform inventory

The three shells are deliberately the same product. Platform specs cover only
mechanics that genuinely differ.

| | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Shell | SwiftUI (`macos/`) | WinUI 3 (`windows/`) | GTK4 (`crates/trace-commons-contributor-gtk`) |
| Background presence | Menu bar extra | Tray icon | StatusNotifierItem tray |
| Same engine | `trace-commons-contributor-ffi` C ABI, hosted in-process on all three | | |

All three have: six-screen onboarding, a main window with **Waiting / History /
Settings**, the preview sheet, the credit record, and the same tray/menu-bar
summary.

## 4. Screen-by-screen, with real copy

### Onboarding — six screens, one decision each

1. **What this is.** Headline "Trace Commons — Contributor". The paragraph
   concedes up front that local scrubbing is "good and it is not perfect —
   which is why you get to look first." A quiet link, **What gets removed?**,
   sits directly under that sentence and opens a sheet listing the detectors:
   "Before a trace leaves this machine, these are found and replaced… Scrubbing
   is pattern-based. It misses things it hasn't seen before."
   Footer promise: **"You decide what gets contributed. Nothing is sent unless
   you say so."** Button: **Get started**.
2. **Connect.** "Paste the invite link someone sent you, or click it from your
   email." Resolves the host before committing: "This invite is for
   **issuer.example.ai**." → **Join issuer.example.ai**. A dead invite says so
   plainly: "This invite link is no longer valid. Ask whoever sent it for a new
   one."
3. **Consent scopes.** "How may your traces be used?" — one always-included
   scope, the rest optional, each one "lets your traces do more." Reversible:
   "You can change this later. It applies to traces you send from now on."
4. **Extra privacy scan (optional).** A choice between **Local scrubbing only**
   and **Local scrubbing + NEAR AI scan** — described honestly as "a second
   scanner run by a third party."
5. **What to watch.** "Which folders may this app watch?" Claude Code and Codex
   roots, each with **Watch this folder**, **Choose a different folder…**, or
   **I don't use this**. Then the project list, each project settable to
   **Ask me first** / **Ignored**.
6. **Done.** "You're set up. Nothing has been sent." Then the login-item ask:
   "Start Trace Commons when you log in? It needs to be running to notice
   finished sessions." — **Not now** / **Start at login**.

### The menu bar / tray

The at-a-glance state, and the only always-visible surface:

- "Nothing waiting" · "3 waiting for your decision" · "Paused." · "Needs
  attention." · "Not watching anything yet"
- Per-project rows with counts and sizes
- "This week: 12 contributed, 1 held for privacy review"
- Pause with real durations: **For 1 hour** / **Until tomorrow morning** /
  **Until I turn it back on** — "Stop noticing finished sessions."
- Actions: **Review waiting sessions…**, **Open Trace Commons**

### Waiting (the queue)

Grouped by project. Each row: opening prompt, size, and how many things the
scrub removed ("3 KB, 4 removed"). Actions per session: **Look inside**
("Opens the redacted preview before deciding"), **Not this one** ("Skips this
session only. This project will keep being offered."), and **Submit all (4)**
for a group. Empty state: "Nothing is waiting."

A "this week" ledger sits alongside: **Contributed** / **Held for privacy
review** / **In the commons**.

Undo is real and honest about its limits: "Not sent. It's back in the queue."
— and when it's too late, "Too late to take that one back — it has already
gone."

### Look inside (the preview sheet) — **the hero shot**

This is the screen that makes the whole product credible. Four tabs:

- **What's in it** — the transcript, paged.
- **Would send** — "Exactly what would be sent," with the banner "Nothing has
  been sent. This is what would be." and the raw size for comparison ("the
  session file on disk is 41 KB").
- **Removed by pattern** — "Scrubbing found: 12 secrets · 4 file paths ·
  2 email addresses", or "Nothing matched a pattern."
- **Permissions** — the consent scopes this envelope would carry.

Plus a local search: "Search this trace for anything you need to be sure isn't
in it." Placeholder: *Client name, hostname, anything*. Reassurance under it:
**"Type to search. Nothing is sent while you look."** (The design note behind
this feature is literally the question "does this mention my client's name?")

Footer: **Not this one** / **Contribute**.

### History

"Everything you've contributed", with three states spelled out in plain words:
**In the commons** / **Held for privacy review** / **Waiting to be scored**.
Each row can be **Withdraw**n. The app refuses to fake precision it doesn't
have: "Typical wait: we don't have a reliable number yet."

### Credit record

Deliberately unglamorous and honest — "Credit, framed honestly." States:
**Recorded** / **Pending review** / **Still being scored** / **Not synced yet**.
The line that governs the tone: *"Still, always. The coin only turns on the
website."*

### Settings

**Watching** (roots and per-project rules), **Projects**, **Connection**
(Connected / Not connected — "Sessions are being queued, but nothing can be
sent."), consent scopes, extra privacy scan, start-at-login, updates ("Checks
daily", **Check Now**, or "Updates managed by Homebrew"), and the public
profile handle opt-in ("List my handle publicly").

## 5. Suggested cut (60–90s)

| # | Beat | On screen | Voiceover / caption |
| --- | --- | --- | --- |
| 1 | The problem | Text on brand black | "Coding agents get better when there are real transcripts to learn from. Almost all of that data is locked inside companies." |
| 2 | The pool | Mint mark, community brand | "Trace Commons is a shared pool that isn't." |
| 3 | Install → onboarding 1 | Welcome screen, cursor pausing on "What gets removed?" | "It watches for finished Claude Code and Codex sessions on your machine." |
| 4 | The concession | The sheet of detectors | "It scrubs them locally. That scrubbing is good, and it's not perfect — which is why you get to look first." |
| 5 | Onboarding 5 → 6 | Roots, then "Nothing has been sent." | "You choose what it may watch." (beat on the sentence, silence) |
| 6 | A session finishes | Terminal → tray badge "1 waiting for your decision" | "When a session finishes, it waits." |
| 7 | **Look inside** | Preview sheet, switching to **Removed by pattern**, chips appearing | "Before you decide, you see exactly what would leave." |
| 8 | The search | Typing a client name → no results | "Search it for anything you need to be sure isn't in there. Nothing is sent while you look." |
| 9 | Contribute | One click, toast | "Then, and only then." |
| 10 | Three platforms | macOS / Windows / Linux windows fanned | "Same app on macOS, Windows and Linux." |
| 11 | Close | Community brand card, black 2px frame | "You decide what gets contributed. Nothing is sent unless you say so." |

Beats 4, 5, 7 and 8 are the ones that cannot be cut. They are properties 1, 3
and 4 from §2. If there's room, add a beat for property 2 (the refusal): a
session that grew after approval, shown being declined.

## 6. Visual direction

The product deliberately contains **two** design languages, and the seam
between them is meaningful — it is the exact boundary of what becomes public.
The video should honour that seam rather than smooth it out.

### The private tool (`TC`)

Warm, quiet, native. Hairline rules, SF/system type, 6–8pt radii, light and
dark.

| Token | Light | Dark |
| --- | --- | --- |
| ground | `#F6F7F4` | `#23251D` |
| surface | `#FFFFFF` | `#21241E` |
| ink primary | `#20241F` | `#E8EAE3` |
| ink secondary | `#5C635B` | `#A6AC9F` |
| line | `#D9DFDC` | `#3B4038` |
| green (good standing) | `#178F70` | `#3FBE9A` |
| gold (held / ranked) | `#B9821F` | `#DCAA43` |
| coral (refused) | `#D65D4F` | `#F2887A` |
| blue (weigh this) | `#315FBA` | `#7FA0EC` |
| redaction chip | `#F3E3C0` on `#202426` | `#4A3C18` on `#F0EBDD` |

### The commons (`CommunityBrand`)

Pure white paper, **2px black frames, no corner radius anywhere**, Helvetica,
mint. Light-only on purpose — a brand panel keeps its own appearance even when
the rest of the window goes dark, which is what makes it read as an embedded
piece of somewhere else.

| Token | Value | Use |
| --- | --- | --- |
| ink | `#000000` | frames, rules, all text in a brand panel |
| paper | `#FFFFFF` | the panel ground |
| accent (mint) | `#00D4AA` | primary button, headline highlight, mark square, coin face |
| rim | `#00B894` | the coin's offset rim; the globe's dashed arc |
| tint | `#EAFAF5` | acknowledgement rows |
| muted | `#6B6B6B` | mono uppercase micro-labels |
| yellow | `#F5C91F` | **exactly one use in the entire product** |

That yellow appears once, on the manifesto headline in the credit record, and
once on the website. A second use is a bug — so if the video wants a highlight
colour, it is mint, not yellow.

## 7. Assets and how to shoot it

### Stills that already exist

`docs/images/` — captured from the real app, light and dark:

```
macos-shell-onboarding-welcome.png      macos-shell-window.png
macos-shell-onboarding-connect.png      macos-shell-menu-bar.png
macos-shell-onboarding-connect-dead-invite.png
macos-shell-consent-scopes.png          macos-shell-preview-sheet.png
macos-shell-onboarding-privacy-scan.png macos-shell-credit-record.png
macos-shell-onboarding-projects.png     macos-shell-onboarding-done.png
```

Dark-appearance variants of the same set are under
`docs/superpowers/plans/macos-design-pass/after-dark/`.

### Live capture (preferred, for motion)

`macos/scripts/run-demo.sh` launches the real app against a throwaway state
directory seeded with two fixture sessions — a **payments-api** session that
contains planted secrets (an AWS key and a GitHub token, so the redaction chips
have something real to show) and a harmless **dotfiles** session. It never
touches the operator's real `~/.claude` or `~/.codex`, and the fixture config
has no device key, so nothing can be uploaded during a shoot.

Relevant env vars the script forwards: `TRACE_COMMONS_SHOW_WINDOW`,
`TRACE_COMMONS_APPEARANCE` (light/dark), `TRACE_COMMONS_DEMO_PREVIEW`,
`TRACE_COMMONS_SCREENSHOT_DIR`.

Windows capture goes through `windows/scripts/win-capture.ps1` against the GCE
dev box described in `windows/docs/dev-vm.md`.

**Never shoot against a real machine's session folder.** Use the fixtures.

## 8. Honesty constraints

The product's whole argument is that it doesn't overclaim, so the video can't
either.

- Say "scrubbed locally", not "anonymised" or "made safe". The app itself says
  scrubbing "misses things it hasn't seen before."
- Credit is not money. Don't animate anything that reads as a token price or a
  payout. "The coin only turns on the website."
- Don't show a notification with an upload action in it. There isn't one.
- Don't invent a wait time for review. The app declines to state one.
- If a shot needs a number (traces contributed, contributors), get a real one
  before it goes in the cut.
