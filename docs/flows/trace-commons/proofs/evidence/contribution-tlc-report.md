# client_contribution

TLC reached more states than the chart budget of 500, so the chart draws only the bug paths: 2 states and 2 arrows.

## P12 breaks
The shortest path has 4 steps:
0. contribution_empty
1. You open history: contribution_history
2. You refresh history: contribution_history
3. The current account changes: contribution_history
4. The history read publishes an empty snapshot: contribution_history

## Rules not checked
P2, P3, P4
After the first break, each TLC run stops at 100,000 distinct states. A run reached that bound before it checked these rules in every state.
