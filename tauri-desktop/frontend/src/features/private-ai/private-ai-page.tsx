import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import { useContributorDisclosureCopy } from "../../lib/tauri/use-contributor-copy";
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
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="PRIVATE / ANSWERS"
        title="Private AI"
        description={disclosure.data?.private_inference.subtitle ?? ""}
        phase="PHASE 4"
      />
      <div className="mb-4 grid grid-cols-3 gap-3 max-[860px]:grid-cols-1">
        <StatCard
          label="Inference access"
          value={
            privateAi.credential?.view?.state_line ??
            privateAi.credential?.state ??
            text(snapshot, "near_ai_inference_configured", "Unknown")
          }
          detail="Credential presence only; never display secrets"
          tone="gold"
        />
        <StatCard
          label="Runtime"
          value={text(runtime, "state", "Unknown")}
          detail={text(runtime, "reason", "Daemon status unavailable")}
          tone="blue"
        />
        <StatCard
          label="Contribution"
          value="Separate"
          detail="Private AI does not publish traces"
        />
      </div>
      {settings.state === "error" && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          Private AI status unavailable. Start Tauri and refresh.
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
      />
      <PrivateAiConnectionPanel privateAi={privateAi} settings={settings} />
      <PrivateAiBalancePanel privateAi={privateAi} />
      <PrivateAiFundingPanel privateAi={privateAi} />
    </div>
  );
}
