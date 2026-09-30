# Native episode UI contract

This contract applies the whole-snapshot episode operations to the macOS,
Windows, and GTK Insights views. Shells use the shared `ui_copy` vocabulary and
the shared service operations. An episode remains a user-selected group of whole
saved snapshots; it is not an inferred task, model result, ranking, cost, or
partial-session selection.

## Presented state and drafts

An open episode freezes its episode ID, last observed revision, and presentation
token. A member-selection draft belongs to that exact presented state. Switching
episodes, closing or hiding Insights, cancelling, or leaving and re-entering the
episode flow invalidates the token and every pending chooser callback.

Create drafts contain selected saved snapshot IDs. Edit drafts start from the
complete current membership and submit the complete replacement set with the
frozen expected revision. Assessment, clear-assessment, and delete requests also
submit that frozen revision. Evidence IDs and revisions may appear in the episode
drilldown; they are not titles or user-visible metrics.

## Completion and reconciliation

Run every operation off the UI thread. A stale callback cannot update the view,
but a mutation already accepted by the service may finish. After any successful
episode mutation, refresh the episode list and its detail from the service. After
a saved snapshot import or deletion, apply the committed cleanup notice first,
then refresh episodes and clear detail for any group that was invalidated. An
automatic refresh failure must not erase that confirmed cleanup notice.

Replacing membership clears the episode assessment. The refreshed detail must
show the group as unassessed and explain that the member change cleared it.
Missing episodes clear stale detail and return to the saved-episodes state.

## Conflicts and failures

On `insights_episode_revision_conflict`, discard the draft, refresh the episode,
and require the user to review it before another edit. Never retry an edit or
deletion automatically. Other failures clear speculative success state and keep
the last confirmed list only when it can still be identified as stale. Empty
history and missing members use their shared empty-state copy and do not create
local state.

Normal snapshot analysis, save, evidence, and assessment controls remain
available outside the episode flow. Episode controls do not depend on account,
enrollment, contribution, discovery, or upload state.
