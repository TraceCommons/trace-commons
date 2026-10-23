import { LegendCell, StatusDot } from "../../design-system";
import {
  PrivateAiBalancePanel,
  useHarnesses,
  usePrivateAi,
  usePrivateAiState,
} from "../../features/private-ai/public";
import { InspectorHeader } from "../../features/waiting/waiting-page";

/**
 * Inspector for the Inference tab: Private AI status, tools, balance. The
 * status is the core's sentence for the reported runtime state; "connected"
 * is the core's harness fact (the tool's config names this destination),
 * never a claim that the tool is answering.
 */
export function InferenceInspector() {
  const privateAi = usePrivateAi();
  const harnesses = useHarnesses();
  const state = usePrivateAiState();
  const rows = harnesses.data?.harnesses ?? [];
  const connected = rows.filter((row) => row.connected);
  return (
    <div className="tc-page">
      <InspectorHeader
        title="Private AI"
        sub={`${connected.length} of ${rows.length} tools connected`}
      />
      <div className="tc-legend">
        <LegendCell color="var(--tc-status-on)" label="connected" value={connected.length} />
        <LegendCell
          color="var(--tc-status-off)"
          label="not connected"
          value={rows.length - connected.length}
        />
      </div>
      <div className="flex flex-col gap-2 px-1 text-[13px]">
        <div className="flex items-start justify-between gap-3">
          <span className="flex-none">Status</span>
          <span className="inline-flex items-start gap-1.5 text-right font-semibold">
            <StatusDot
              tone={state.working ? "on" : "outside"}
              size="md"
              className="mt-1"
            />
            {state.line ?? "—"}
          </span>
        </div>
        <div className="flex items-start justify-between gap-3">
          <span>Credential</span>
          <span className="text-right font-semibold">
            {privateAi.credential?.view?.state_line ??
              privateAi.credential?.state ??
              "—"}
          </span>
        </div>
        <div className="flex items-start justify-between gap-3">
          <span>Connected tools</span>
          <span className="text-right font-semibold">
            {connected.length
              ? connected.map((row) => row.name).join(", ")
              : "None"}
          </span>
        </div>
      </div>
      <PrivateAiBalancePanel privateAi={privateAi} />
    </div>
  );
}
