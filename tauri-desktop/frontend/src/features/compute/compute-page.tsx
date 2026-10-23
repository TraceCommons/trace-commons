import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import { ComputePanel } from "./components/compute-panel";
import { useComputeStatus } from "./hooks/use-compute-status";

export function ComputePage() {
  const compute = useComputeStatus();
  const snapshot = compute.data;
  return (
    <div className="tc-page">
      <PageHeader
        eyebrow="LOCAL / CAPACITY"
        title="Compute"
        description="Use local capacity for private model work when it is available and explicitly enabled."
        phase="PHASE 4"
      />
      <div className="mb-2.5 grid grid-cols-3 gap-1.5">
        <StatCard
          label="State"
          value={snapshot?.state ?? "Unknown"}
          detail={snapshot?.reason ?? "Compute monitor not connected"}
          tone={snapshot?.state === "unavailable" ? "gold" : "blue"}
        />
        <StatCard
          label="Consent"
          value={snapshot?.consent_granted ? "Granted" : "Required"}
          detail="Separate from trace contribution"
        />
        <StatCard
          label="Worker"
          value={snapshot?.available ? "Available" : "Unavailable"}
          detail="No worker starts automatically"
          tone="gold"
        />
      </div>
      <ComputePanel compute={compute} />
    </div>
  );
}
