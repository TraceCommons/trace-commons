import type { BehaviorSetting } from "../api/behavior-api";
import { BehaviorSettingRow } from "./behavior-setting-row";
import { TertiaryLink } from "@/design-system";

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
    <section className="tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            BEHAVIOR
          </span>
          <h2>How contribution behaves</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy !== null}
        >
          Refresh
        </TertiaryLink>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        These controls change local timing and hard upload limits. They do not
        change consent or project policy.
      </p>
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      <div className="mt-3 grid gap-px">
        <BehaviorSettingRow
          label="Finished-trace quiet period"
          detail="Time without new events before a trace enters Waiting."
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
      <div className="mt-3 grid gap-px">
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
