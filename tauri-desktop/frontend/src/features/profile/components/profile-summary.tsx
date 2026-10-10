import type { CoreStatus, CoreStatusState } from "../../../lib/tauri/types";
import type { PublicProfile } from "../types";
import { TertiaryLink } from "@/design-system";

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
    <section className="tc-card">
      <div className="grid grid-cols-[auto_1fr_auto] items-center gap-[18px] max-[860px]:grid-cols-[auto_1fr]">
        <div className="tc-card grid h-[66px] w-[66px] place-items-center text-[19px] font-extrabold text-tc-on-accent">
          TC
        </div>
        <div>
          <span className="mb-1.5 block tc-eyebrow">
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
        <span className="tc-chip tc-chip--glass self-start">
          {connected ? "Rust core connected" : "Connecting"}
        </span>
      </div>
      <div className="mt-7 grid grid-cols-3 gap-px border-y border-tc-hairline">
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
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={coreStatusState === "loading"}
        >
          Refresh status
        </TertiaryLink>
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
