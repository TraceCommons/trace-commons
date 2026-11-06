// Server panels are explanatory review frames, not proposed application UI.
export const title = "Receipt acceptance and routing";
export function state(v: any): string {
  const s = v.s ?? v;
  return s.frame;
}
export function step(action: string, _from: any, _to: any): string {
  return action.replace(/([a-z])([A-Z])/g, "$1 $2");
}
