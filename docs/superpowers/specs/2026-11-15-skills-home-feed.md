# Skills Home feed

Home becomes the default Skills tab. It shows changes rather than summary counters: updates for installed skills the person uses, changes in skill usage, and recent catalog changes from followed creators. Each View action opens the exact catalog skill or usage chart. Index, Usage, Catalog, and Following remain available.

Home reads as a feed, not a dashboard (owner, 2026-11-15): one column of story cards, newest first, under Today, This week (the local ISO week), and Earlier, not a card per category. A story is a mark, one sentence naming the skill, where it comes from, a short description when the catalog has one, and one View action. Only a catalog change has a time of its own and shows it; an available update and a usage change say what is true now, so they sit under Today and show no event time (a usage change shows its period). A catalog change for a skill whose update is already in the feed is told once, as the update. The column stops at 560pt on a wide pane.

Second pass (owner, 2026-11-15, each item approved by name):

- Covers. A catalog skill has a cover drawn in the app from its id and its publisher's id: the publisher picks one of eight gradients, the skill picks a pattern (rings, bars, dots, arcs) and where it sits, and the publisher's initials are set on it. The seed is FNV-1a with a finalizer, never `Hasher`, so a cover is the same on every run. Covers are not re-coloured for light or dark, hold white at 4.5:1 on every stop, and are hidden from assistive technology.
- Hero. The first story under Today is drawn tall under its cover when it is about a catalog skill. A usage change, a roll-up, or a feed with nothing today has no hero; nothing is reordered to make one.
- Usage chart. A usage change shows its fourteen completed local days as bars, the last seven in the agent's tint, with both weeks' totals in words under it.
- Roll-ups. Two or more catalog changes from one publisher in one period are one card with a strip of covers, each opening its skill.
- Your week. The three most-used skills of the past seven days open This week, each row opening its usage. Left out while usage history is partial.
- Motion. Cards rise slightly under the pointer and ease in on scroll, bars rise once, counts roll, and a story's description opens in place. All of it is off under Reduce Motion.

Not approved, and not built: unread dots, "New" markers, badges, counts on the tab, streaks, milestone celebrations. Trending (public GitHub signals) and Popular this week (a server list) are separate modules that are simply absent without their data.

Use existing Insights typography, cards, and agent logos. Keep the shared copy brief. Omit empty periods; when the whole feed is empty, show one neutral empty state with Installed skills and Browse actions. No Activity chart, new selectors, related traces, notices, or automatic installation.

Updates require verified installed/catalog source matching and known recent uses. Trends compare the last seven completed local days to the previous seven, combining only disjoint explicit invocation evidence. Incomplete or ambiguous history cannot imply a decrease or a first-ever use. Creator updates are catalog changes, not fabricated creator posts.

All data remains local or from the existing bounded public catalog feed. Feature flags, consent, folder rules, fetching, and backend contracts do not change. A stale target must never open a different skill.
