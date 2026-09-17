import { Button } from "@/components/ui/button";
import type { HarnessList, HarnessPlan } from "../api/harness-api";

export function HarnessListPanel({
  data,
  state,
  plan,
  actionState,
  error,
  onRefresh,
  onPlan,
  onCommit,
  onCancel,
}: {
  data: HarnessList | null;
  state: "loading" | "ready" | "error";
  plan: HarnessPlan | null;
  actionState: "idle" | "busy" | "error";
  error: string | null;
  onRefresh: () => Promise<void>;
  onPlan: (id: string, action: "connect" | "disconnect") => Promise<void>;
  onCommit: () => Promise<void>;
  onCancel: () => void;
}) {
  const rows = data?.harnesses ?? [];
  const view = data?.view;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            LOCAL TOOLS
          </span>
          <h2>{data?.view.title ?? "Configured tools"}</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={state === "loading"}
        >
          Refresh
        </Button>
      </div>
      {view?.what && <p>{view.what}</p>}
      {view?.spend_line && (
        <>
          <p>{view.spend_line}</p>
          <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
            {view.spend_scope}
          </p>
        </>
      )}
      {view?.credential_notice && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          {view.credential_notice}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Reading configured tools…
        </p>
      )}
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {state === "error" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Configured tools unavailable. Refresh after Rust core starts.
        </p>
      )}
      {state === "ready" && rows.length === 0 && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          {view?.none_found}
        </p>
      )}
      {state === "ready" && rows.length > 0 && (
        <div className="mt-[22px] grid gap-px border-t border-border">
          {rows.map((row) => (
            <div
              className="flex justify-between gap-[18px] border-b border-border py-[15px]"
              key={row.id}
            >
              <div>
                <strong>{row.name}</strong>
                <span>
                  {row.state_line ||
                    (row.installed
                      ? "Installed; no attributed activity"
                      : "Not installed")}
                </span>
                {row.config_path && <code>{row.config_path}</code>}
              </div>
              <div className="flex flex-wrap items-center justify-end gap-2">
                <span className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
                  {row.connected
                    ? "Connected to local destination"
                    : "Not connected"}
                </span>
                {row.can_connect && (
                  <Button
                    className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
                    type="button"
                    onClick={() => void onPlan(row.id, "connect")}
                    disabled={actionState === "busy"}
                  >
                    Connect
                  </Button>
                )}
                {row.can_disconnect && (
                  <Button
                    className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
                    type="button"
                    onClick={() => void onPlan(row.id, "disconnect")}
                    disabled={actionState === "busy"}
                  >
                    Disconnect
                  </Button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
      {plan && (
        <div className="mt-[18px] grid gap-[9px] rounded-[10px] border border-primary/20 bg-primary/5 p-[18px]">
          <div className="flex items-start justify-between gap-[18px]">
            <div>
              <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                PREVIEW
              </span>
              <h3>{plan.view.preview_title}</h3>
            </div>
            <Button
              className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
              type="button"
              onClick={onCancel}
              disabled={actionState === "busy"}
            >
              Cancel
            </Button>
          </div>
          {plan.path && <code>{plan.path}</code>}
          {plan.view.outcome_line && <p>{plan.view.outcome_line}</p>}
          {plan.changes.map((change) => (
            <code key={change}>{change}</code>
          ))}
          {plan.occupied.length > 0 && (
            <>
              <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
                {plan.view.slot_taken}
              </p>
              {plan.occupied.map((slot) => (
                <code key={slot.slot}>
                  {slot.slot}: {slot.current}
                </code>
              ))}
            </>
          )}
          {plan.view.can_commit && (
            <Button
              className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
              type="button"
              onClick={() => void onCommit()}
              disabled={actionState === "busy"}
            >
              {plan.view.confirm}
            </Button>
          )}
        </div>
      )}
    </section>
  );
}
