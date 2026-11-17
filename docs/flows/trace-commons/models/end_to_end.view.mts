// Abstract composition: presentation projects visible history, never current server phase.
export const title = "TraceCommons abstract composition";
const unwrap = (v: any): any => v?.s ?? v?.vars?.s ?? v?.vars ?? v;
export function state(vars: any): string {
  const s = unwrap(vars);
  const entries = s.traces;
  const t = entries instanceof Map ? entries.get(s.selected) : Array.isArray(entries)
    ? entries.find((e: any) => Array.isArray(e) && Number(e[0]) === s.selected)?.[1] ?? entries[s.selected]
    : entries[s.selected] ?? entries[String(s.selected)];
  if (!s.app) return "e2e_stopped";
  if (!s.window) return "e2e_closed";
  if (!s.daemon) return "e2e_offline";
  if (s.account === -1) return "e2e_signed_out";
  if (t.visible === "withdrawn" || t.visible === "revoked") return "e2e_withdrawn";
  if (t.visibleCredit > 0) return "e2e_credit";
  if (t.localReceipt) return "e2e_history";
  if (t.queue === "sending") return "e2e_sending";
  if (t.queue === "retry") return "e2e_retry";
  if (t.queue === "approved") return "e2e_approved";
  if (t.preview) return "e2e_preview";
  return "e2e_pending";
}
export function step(action: string, _from: any, _to: any): string {
  return action.replace(/([a-z])([A-Z])/g, "$1 $2");
}
