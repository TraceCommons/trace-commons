import { Button } from "@/components/ui/button";
import type { BehaviorSetting } from "../api/behavior-api";
import { BehaviorSettingRow } from "./behavior-setting-row";

function numberValue(
  settings: Record<string, unknown>,
  key: string,
  fallback: number,
) {
  return typeof settings[key] === "number" && Number.isFinite(settings[key])
    ? (settings[key] as number)
    : fallback;
}

export function BehaviorSettingsPanel({
  settings,
  busy,
  error,
  onRefresh,
  onSave,
}: {
  settings: Record<string, unknown>;
  busy: BehaviorSetting | null;
  error: string | null;
  onRefresh: () => Promise<void>;
  onSave: (setting: BehaviorSetting, value: number) => Promise<unknown>;
}) {
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            BEHAVIOR
          </span>
          <h2>How contribution behaves</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy !== null}
        >
          Refresh
        </Button>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        These controls change local timing and hard upload limits. They do not
        change consent or project policy.
      </p>
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      <div className="mt-5 grid gap-px border-t border-border">
        <BehaviorSettingRow
          label="Finished-session quiet period"
          detail="Time without new events before a session enters Waiting."
          setting="quiescence"
          value={Math.round(numberValue(settings, "quiescence_secs", 300) / 60)}
          min={1}
          max={240}
          unit="minutes"
          busy={busy === "quiescence"}
          onSave={onSave}
        />
        <BehaviorSettingRow
          label="Approval undo window"
          detail="Hold after approval before uploader may send."
          setting="approval_hold"
          value={numberValue(settings, "approval_hold_secs", 30)}
          min={0}
          max={300}
          unit="seconds"
          busy={busy === "approval_hold"}
          onSave={onSave}
        />
        <BehaviorSettingRow
          label="Digest interval"
          detail="Minimum time between local notifications."
          setting="digest"
          value={Math.round(
            numberValue(settings, "digest_interval_secs", 21600) / 3600,
          )}
          min={1}
          max={24}
          unit="hours"
          busy={busy === "digest"}
          onSave={onSave}
        />
      </div>
      <div className="mt-5 grid gap-px border-t border-border">
        <BehaviorSettingRow
          label="Daily upload count"
          detail="Hard maximum accepted by daemon."
          setting="max_uploads"
          value={numberValue(settings, "max_uploads_per_day", 100)}
          min={1}
          max={1000}
          unit="uploads"
          busy={busy === "max_uploads"}
          onSave={onSave}
        />
        <BehaviorSettingRow
          label="Daily upload volume"
          detail="Hard maximum accepted by daemon."
          setting="max_bytes"
          value={Math.max(
            1,
            Math.round(
              numberValue(settings, "max_bytes_per_day", 512 * 1024 * 1024) /
                1024 /
                1024,
            ),
          )}
          min={1}
          max={5120}
          unit="MB"
          busy={busy === "max_bytes"}
          onSave={onSave}
        />
      </div>
    </section>
  );
}
