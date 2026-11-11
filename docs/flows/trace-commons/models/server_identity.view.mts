// Explanatory server panels, not proposed application UI.
export const title = "Server sign-in, sessions and device authority";
export function state(v: any): string {
  return (v.s ?? v).frame;
}
export function step(action: string, _from: any, _to: any): string {
  return action.replace(/([a-z])([A-Z])/g, "$1 $2");
}
