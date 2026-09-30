import { LegendCell, StatusDot } from "../../design-system";
import {
  PrivateAiBalancePanel,
  useHarnesses,
  usePrivateAi,
} from "../../features/private-ai/public";
import { useSettings } from "../../features/settings/public";
import { InspectorHeader } from "../../features/waiting/waiting-page";

/** Inspector for the Inference tab: Private AI status, tools, balance. */
export function InferenceInspector() {
  const settings = useSettings();
  const privateAi = usePrivateAi();
  const harnesses = useHarnesses();
  const on = settings.data?.private_inference === true;
  const rows = harnesses.data?.harnesses ?? [];
  const connected = rows.filter((row) => row.connected);
  return (
    <div className="tc-page">
      <InspectorHeader
        title="Private AI"
        sub={
          on
            ? `On · ${connected.length} of ${rows.length} tools answer on NEAR AI`
            : "Off · tools answer at their vendors"
        }
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
        <div className="flex items-center justify-between gap-3">
          <span>Status</span>
          <span className="inline-flex items-center gap-1.5 font-semibold">
            <StatusDot tone={on ? "on" : "outside"} size="md" />
            {on ? "On" : "Off"}
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
