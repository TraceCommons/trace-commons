import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useState } from "react";
import type { TokenStorage } from "../api/privacy-api";

type PrivacyAction = "inference" | "token" | "capture" | "discard";

export function PrivacyControlsPanel({
  settings,
  storage,
  state,
  error,
  onRefresh,
  onInference,
  onToken,
  onCapture,
  onCleanup,
}: {
  settings: Record<string, unknown>;
  storage: TokenStorage | null;
  state: "loading" | "ready" | "error" | "busy";
  error: string | null;
  onRefresh: () => Promise<void>;
  onInference: (enabled: boolean, confirmed: boolean) => Promise<void>;
  onToken: (enabled: boolean, confirmed: boolean) => Promise<void>;
  onCapture: (enabled: boolean, confirmed: boolean) => Promise<void>;
  onCleanup: (discard: boolean, confirmed: boolean) => Promise<void>;
}) {
  const [confirming, setConfirming] = useState<PrivacyAction | null>(null);
  const busy = state === "busy" || state === "loading";
  const ask = (
    action: PrivacyAction,
    enabled: boolean,
    run: (confirmed: boolean) => Promise<void>,
  ) => {
    if (!enabled) void run(false);
    else setConfirming(action);
  };
  const confirm = (run: () => Promise<void>) => {
    setConfirming(null);
    void run();
  };
  return (
    <section className="tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            PRIVACY EVIDENCE
          </span>
          <h2>Optional local evidence</h2>
        </div>
        <Button
          className="tc-link"
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy}
        >
          Refresh
        </Button>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Each option is separate from contribution consent. Enabling requires
        reading its disclosure; Rust confirms the setting.
      </p>
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      <div className="mt-3 grid gap-px">
        <PrivacyToggle
          label="Model-call evidence"
          detail="Keep attestation evidence with locally reviewed model calls."
          enabled={settings.ironwire_attested_bodies === true}
          disabled={busy}
          onChange={(enabled) =>
            ask("inference", enabled, (confirmed) =>
              onInference(enabled, confirmed),
            )
          }
        />
        {confirming === "inference" && (
          <Disclosure
            text="This records model-call evidence for local admission review. It does not publish raw prompts."
            onConfirm={() => confirm(() => onInference(true, true))}
            onCancel={() => setConfirming(null)}
          />
        )}
        <PrivacyToggle
          label="Token distribution contribution"
          detail="Allow eligible token-derived evidence to be considered separately."
          enabled={settings.token_distributions_contribution === true}
          disabled={busy}
          onChange={(enabled) =>
            ask("token", enabled, (confirmed) => onToken(enabled, confirmed))
          }
        />
        {confirming === "token" && (
          <Disclosure
            text="Token-derived data has separate retention and contribution implications. Review before enabling."
            onConfirm={() => confirm(() => onToken(true, true))}
            onCancel={() => setConfirming(null)}
          />
        )}
        <PrivacyToggle
          label="Local token capture"
          detail={storage?.capture_notice ?? "Status not read."}
          enabled={storage?.capture_enabled === true}
          disabled={busy || !storage}
          onChange={(enabled) =>
            ask("capture", enabled, (confirmed) =>
              onCapture(enabled, confirmed),
            )
          }
        />
        {confirming === "capture" && storage && (
          <Disclosure
            text={storage.capture_confirmation}
            onConfirm={() => confirm(() => onCapture(true, true))}
            onCancel={() => setConfirming(null)}
          />
        )}
      </div>
      {storage && (
        <div className="mt-5 grid gap-[6px] border-t border-border pt-5">
          <strong>Local token-review storage</strong>
          <span>{storage.state_line}</span>
          <span>{storage.scope_note}</span>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              className="tc-btn tc-btn--glass"
              type="button"
              onClick={() => void onCleanup(false, false)}
              disabled={busy}
            >
              {storage.cleanup_label}
            </Button>
            <Button
              className="tc-btn tc-btn--glass tc-text-outside"
              type="button"
              onClick={() => setConfirming("discard")}
              disabled={busy}
            >
              {storage.discard_label}
            </Button>
          </div>
        </div>
      )}
      {confirming === "discard" && storage && (
        <Disclosure
          text={storage.discard_confirmation}
          onConfirm={() => confirm(() => onCleanup(true, true))}
          onCancel={() => setConfirming(null)}
        />
      )}
    </section>
  );
}

function PrivacyToggle({
  label,
  detail,
  enabled,
  disabled,
  onChange,
}: {
  label: string;
  detail: string;
  enabled: boolean;
  disabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-[18px] border-b border-border py-[15px]">
      <div>
        <strong>{label}</strong>
        <span>{detail}</span>
      </div>
      <Button
        variant={enabled ? "secondary" : "default"}
        type="button"
        onClick={() => onChange(!enabled)}
        disabled={disabled}
      >
        {enabled ? "Disable" : "Enable"}
      </Button>
    </div>
  );
}

function Disclosure({
  text,
  onConfirm,
  onCancel,
}: {
  text: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <ResponsiveOverlay
      open
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title="Review privacy change"
      description="Changing local privacy behavior requires explicit confirmation."
      footer={
        <>
          <Button type="button" variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button type="button" onClick={onConfirm}>
            I understand — enable
          </Button>
        </>
      }
    >
      <p className="text-sm leading-6 text-muted-foreground">{text}</p>
    </ResponsiveOverlay>
  );
}
