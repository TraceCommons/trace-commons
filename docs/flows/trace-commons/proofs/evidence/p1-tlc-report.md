# client_consent

TLC reached more states than the chart budget of 500, so the chart draws only the bug paths: 2 states and 2 arrows.

## P1 breaks
The shortest path has 10 steps:
0. consent_choice
1. You inspect consent internals: consent_internal
2. You start the first run sharing commit: consent_internal
3. The standing grants adopt chosen scopes: consent_internal
4. The scope choice is written: consent_internal
5. The first run reaches its grant: consent_internal
6. The grant audit is recorded: consent_internal
7. The fresh grant is saved: consent_internal
8. The sharing choice requests the saved grant: consent_internal
9. You withdraw the grant: consent_internal
10. The sharing choice receives the saved grant: consent_internal
