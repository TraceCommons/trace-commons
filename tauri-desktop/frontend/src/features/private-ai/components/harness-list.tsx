import type { HarnessList, HarnessPlan } from "../api/harness-api";
import { ButtonPrimary, GlassButton, TertiaryLink } from "@/design-system";

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
    <section className="tc-card mb-2.5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            LOCAL TOOLS
          </span>
          <h2>{data?.view.title ?? "Configured tools"}</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={state === "loading"}
        >
          Refresh
        </TertiaryLink>
      </div>
      {view?.what && <p>{view.what}</p>}
      {view?.spend_line && (
        <>
          <p>{view.spend_line}</p>
          <p className="m-0 tc-caption tc-text-tertiary">
            {view.spend_scope}
          </p>
        </>
      )}
      {view?.credential_notice && (
        <p className="m-0 tc-caption tc-text-tertiary">
          {view.credential_notice}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Reading configured tools…
        </p>
      )}
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      {state === "error" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Configured tools unavailable. Refresh after Rust core starts.
        </p>
      )}
      {state === "ready" && rows.length === 0 && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          {view?.none_found}
        </p>
      )}
      {state === "ready" && rows.length > 0 && (
        <div className="mt-3 grid gap-px">
          {rows.map((row) => (
            <div
              className="flex justify-between gap-[18px] border-b border-tc-hairline py-[15px]"
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
                <span className="m-0 tc-caption tc-text-tertiary">
                  {row.connected
                    ? "Connected to local destination"
                    : "Not connected"}
                </span>
                {row.can_connect && (
                  <GlassButton
                    type="button"
                    onClick={() => void onPlan(row.id, "connect")}
                    disabled={actionState === "busy"}
                  >
                    Connect
                  </GlassButton>
                )}
                {row.can_disconnect && (
                  <GlassButton
                    className="tc-text-outside"
                    type="button"
                    onClick={() => void onPlan(row.id, "disconnect")}
                    disabled={actionState === "busy"}
                  >
                    Disconnect
                  </GlassButton>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
      {plan && (
        <div className="tc-card tc-card--quiet mt-[18px] grid gap-[9px]">
          <div className="flex items-start justify-between gap-3">
            <div>
              <span className="mb-1.5 block tc-eyebrow">
                PREVIEW
              </span>
              <h3>{plan.view.preview_title}</h3>
            </div>
            <TertiaryLink
              type="button"
              onClick={onCancel}
              disabled={actionState === "busy"}
            >
              Cancel
            </TertiaryLink>
          </div>
          {plan.path && <code>{plan.path}</code>}
          {plan.view.outcome_line && <p>{plan.view.outcome_line}</p>}
          {plan.changes.map((change) => (
            <code key={change}>{change}</code>
          ))}
          {plan.occupied.length > 0 && (
            <>
              <p className="m-0 tc-caption tc-text-tertiary">
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
            <ButtonPrimary size="sm"
              type="button"
              onClick={() => void onCommit()}
              disabled={actionState === "busy"}
            >
              {plan.view.confirm}
            </ButtonPrimary>
          )}
        </div>
      )}
    </section>
  );
}
