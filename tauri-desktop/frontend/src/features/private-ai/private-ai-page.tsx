import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import {
  useContributorDisclosureCopy,
  useShellStatusLines,
} from "../../lib/tauri/use-contributor-copy";
import { useSettings } from "../settings/public";
import { HarnessListPanel } from "./components/harness-list";
import { PrivateAiBalancePanel } from "./components/private-ai-balance-panel";
import { PrivateAiConnectionPanel } from "./components/private-ai-connection-panel";
import { PrivateAiFundingPanel } from "./components/private-ai-funding-panel";
import { useHarnesses } from "./hooks/use-harnesses";
import { usePrivateAi } from "./hooks/use-private-ai";

function text(
  settings: Record<string, unknown> | null,
  key: string,
  fallback: string,
) {
  const value = settings?.[key];
  return value === undefined || value === null ? fallback : String(value);
}

export function PrivateAiPage() {
  const lines = useShellStatusLines();
  const settings = useSettings();
  const privateAi = usePrivateAi();
  const harnesses = useHarnesses();
  const disclosure = useContributorDisclosureCopy();
  const snapshot = settings.data;
  const runtime =
    typeof snapshot?.private_inference_state === "object" &&
    snapshot.private_inference_state !== null
      ? (snapshot.private_inference_state as Record<string, unknown>)
      : null;
  return (
    <div className="tc-page">
      <PageHeader
        title="Private AI"
        description={disclosure.data?.private_inference.subtitle ?? ""}
        titleHidden
      />
      <div className="grid grid-cols-2 gap-1.5">
        <StatCard
          label="Inference access"
          value={
            privateAi.credential?.view?.state_line ??
            privateAi.credential?.state ??
            text(snapshot, "near_ai_inference_configured", "Unknown")
          }
        />
        <StatCard label="Runtime" value={text(runtime, "state", "Unknown")} />
      </div>
      {settings.state === "error" && (
        <p className="tc-alert">
          {lines.readUnavailable}
        </p>
      )}
      <HarnessListPanel
        data={harnesses.data}
        state={harnesses.state}
        plan={harnesses.plan}
        actionState={harnesses.actionState}
        error={harnesses.error}
        onRefresh={harnesses.refresh}
        onPlan={harnesses.prepare}
        onCommit={harnesses.commit}
        onCancel={harnesses.cancel}
        readUnavailable={lines.readUnavailable}
      />
      <PrivateAiConnectionPanel privateAi={privateAi} settings={settings} />
      <PrivateAiBalancePanel privateAi={privateAi} />
      <PrivateAiFundingPanel privateAi={privateAi} />
    </div>
  );
}
