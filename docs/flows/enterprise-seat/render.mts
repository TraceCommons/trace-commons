// Render-only view for the merged chart: the check's view, with steps the drawing hides.
// Every hidden step still runs in the model and is checked; the README names them.
import { stepLabels } from "./labels.mts";
export { title, state, group, colors } from "./view.mts";

export const hidden = new Set([
  "YouChooseAutomatic", "YouChooseAsk", "YourMacFetchesPolicy",
  "NearAiRemovesYouFromTheOrg", "NearAiAddsYouBack",
  "TheAdminForbidsAutomatic", "TheAdminAllowsAutomatic", "TheAdminAcceptsServerTerms",
  "TheCreditSettles", "TheClaimRequestFails", "TheServerSharesATrace", "TheServerTraceIsCredited", "TheServerTraceIsRefused",
]);
export const step = (action: string) => (hidden.has(action) ? null : stepLabels[action] ?? action);
