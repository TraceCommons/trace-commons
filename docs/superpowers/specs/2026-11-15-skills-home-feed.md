# Skills Home feed

Home becomes the default Skills tab. It shows changes rather than summary counters: updates for installed skills the person uses, changes in skill usage, and recent catalog changes from followed creators. Each View action opens the exact catalog skill or usage chart. Index, Usage, Catalog, and Following remain available.

Home reads as a feed, not a dashboard (owner, 2026-11-15): one column of story cards, newest first, under Today, This week (the local ISO week), and Earlier, not a card per category. A story is a mark, one sentence naming the skill, where it comes from, a short description when the catalog has one, and one View action. Only a catalog change has a time of its own and shows it; an available update and a usage change say what is true now, so they sit under Today and show no event time (a usage change shows its period). A catalog change for a skill whose update is already in the feed is told once, as the update. The column stops at 560pt on a wide pane.

Use existing Insights typography, cards, and agent logos. Keep the shared copy brief. Omit empty periods; when the whole feed is empty, show one neutral empty state with Installed skills and Browse actions. No Activity chart, new selectors, related traces, notices, or automatic installation.

Updates require verified installed/catalog source matching and known recent uses. Trends compare the last seven completed local days to the previous seven, combining only disjoint explicit invocation evidence. Incomplete or ambiguous history cannot imply a decrease or a first-ever use. Creator updates are catalog changes, not fabricated creator posts.

All data remains local or from the existing bounded public catalog feed. Feature flags, consent, folder rules, fetching, and backend contracts do not change. A stale target must never open a different skill.
