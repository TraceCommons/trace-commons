// Explanatory server panels; these controls are model events, not application UI.
export const title = "Settlement and separate payout rails";
export function state(v: any): string { return (v.s ?? v).panel; }
export function step(action: string, _from: any, _to: any): string {
  return action.replace(/([a-z])([A-Z])/g, "$1 $2");
}
