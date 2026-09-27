import { Button } from "@/components/ui/button";
import type { CoreStatus, CoreStatusState } from "../../../lib/tauri/types";
import type { PublicProfile } from "../types";

type ProfileSummaryProps = {
  coreStatus: CoreStatus | null;
  coreStatusState: CoreStatusState;
  onRefresh: () => Promise<void>;
  profile: PublicProfile | null;
  profileState: "loading" | "ready" | "error";
};

export function ProfileSummary({
  coreStatus,
  coreStatusState,
  onRefresh,
  profile,
  profileState,
}: ProfileSummaryProps) {
  const connected = coreStatusState === "ready" && coreStatus !== null;
  const daemon = coreStatus?.daemon;
  const published = profileState === "ready" && profile?.on_roster === true;

  return (
    <section className="p-7 rounded-2xl border border-border bg-card/80">
      <div className="grid grid-cols-[auto_1fr_auto] items-center gap-[18px] max-[860px]:grid-cols-[auto_1fr]">
        <div className="grid h-[66px] w-[66px] place-items-center rounded-full bg-primary text-[19px] font-extrabold text-primary-foreground shadow-[0_0_0_8px_var(--color-green-soft)]">
          TC
        </div>
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            PUBLIC PROFILE
          </span>
          <h2>
            {published
              ? `@${profile?.handle ?? "unnamed"}`
              : "Not published yet"}
          </h2>
          <p>
            {published
              ? profile?.bio || "Published contributor profile"
              : "Your profile stays local until you explicitly connect and publish it."}
          </p>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          {connected ? "Rust core connected" : "Connecting"}
        </span>
      </div>
      <div className="mt-7 grid grid-cols-3 gap-px border-y border-border">
        <SummaryItem
          label="Handle"
          value={published ? `@${profile?.handle ?? "—"}` : "Not claimed"}
        />
        <SummaryItem
          label="Contribution state"
          value={daemon?.logged_in ? "Enrolled" : "Local only"}
        />
        <SummaryItem
          label="Queue"
          value={`${daemon?.queue_depth ?? "—"} waiting`}
        />
      </div>
      <div className="flex items-center justify-between gap-[18px] pt-[17px]">
        <span>
          {coreStatusState === "error"
            ? "Rust core status unavailable"
            : "Status read from existing contributor daemon"}
        </span>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={coreStatusState === "loading"}
        >
          Refresh status
        </Button>
      </div>
    </section>
  );
}

function SummaryItem({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
