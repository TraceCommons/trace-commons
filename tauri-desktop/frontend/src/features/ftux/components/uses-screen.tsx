import { groupState, OPTIONAL_USES, optionalUsesLabel } from "../ftux-model";
import type { SharingMode } from "../types";
import {
  ChevronIcon,
  GlassCheckbox,
  GlassSwitch,
  PillSelect,
  ScreenTitle,
  type SelectOption,
  Spinner,
} from "./glass";

const SHARING_OPTIONS: SelectOption<SharingMode>[] = [
  { value: "auto", label: "Share automatically", tone: "green" },
  { value: "ask", label: "Ask me each time", tone: "yellow" },
];

// W-3 (Connect and forget) and W-6 (Customize and tailor, which adds the
// Private AI switch).
export function UsesScreen({
  optionalUses,
  listHandle,
  sharing,
  privateAi,
  submitting,
  error,
  onToggleUses,
  onToggleUse,
  onToggleHandle,
  onSharing,
  onTogglePrivateAi,
  onStart,
}: {
  optionalUses: boolean[];
  listHandle: boolean;
  sharing: SharingMode;
  // Null hides the Private AI card (Connect and forget).
  privateAi: boolean | null;
  submitting: boolean;
  error: string | null;
  onToggleUses: () => void;
  onToggleUse: (index: number) => void;
  onToggleHandle: () => void;
  onSharing: (mode: SharingMode) => void;
  onTogglePrivateAi: () => void;
  onStart: () => void;
}) {
  const usesState = groupState(optionalUses);
  return (
    <>
      <ScreenTitle light="How your data is " bold="used & permissioned." />
      <div className="ftux-scroll">
        <div className="ftux-card">
          <span className="ftux-section-label">
            How your traces may be used
          </span>
          <div className="ftux-check-row">
            <GlassCheckbox
              checked
              disabled
              label="Finding bugs and measuring agents, always on"
            />
            <span style={{ flex: 1 }}>
              <strong>Finding bugs and measuring agents</strong>
              <span className="ftux-always-on">always on</span>
              <br />
              <span className="ftux-muted">
                Researchers read traces to see where coding agents fail and to
                score agents against each other.
              </span>
            </span>
          </div>
          <div className="ftux-check-row">
            <GlassCheckbox
              checked={
                usesState === "all"
                  ? true
                  : usesState === "some"
                    ? "mixed"
                    : false
              }
              label="All optional uses"
              onToggle={onToggleUses}
            />
            <details className="ftux-disclosure">
              <summary>
                <strong style={{ flex: 1 }}>
                  {optionalUsesLabel(optionalUses)}
                </strong>
                <ChevronIcon size={12} />
              </summary>
              <div
                className="ftux-disclosure-body"
                style={{ gap: 8, paddingTop: 8 }}
              >
                {OPTIONAL_USES.map((use, index) => (
                  <div key={use} className="ftux-check-row">
                    <GlassCheckbox
                      checked={optionalUses[index] === true}
                      label={use}
                      onToggle={() => onToggleUse(index)}
                    />
                    <strong style={{ flex: 1 }}>{use}</strong>
                  </div>
                ))}
              </div>
            </details>
          </div>
          <div className="ftux-check-row">
            <GlassCheckbox
              checked={listHandle}
              label="List my handle publicly as a contributor"
              onToggle={onToggleHandle}
            />
            <span style={{ flex: 1 }}>
              <strong>List my handle publicly as a contributor</strong>
              <br />
              <span className="ftux-muted">
                Credit only. It does not change how any trace is used.
              </span>
            </span>
          </div>
        </div>

        <div className="ftux-card">
          <div className="ftux-card-row">
            <span style={{ display: "flex", flexDirection: "column" }}>
              <span className="ftux-card-title">Sharing</span>
              <span className="ftux-card-text">
                Scrubbed on this device, shared once quality checked.
                Undetermined sessions wait for your approval.
              </span>
            </span>
            <PillSelect
              label="Sharing"
              value={sharing}
              options={SHARING_OPTIONS}
              onChange={onSharing}
            />
          </div>
        </div>

        {privateAi === null ? null : (
          <div className="ftux-card">
            <div className="ftux-card-row">
              <span style={{ display: "flex", flexDirection: "column" }}>
                <span className="ftux-card-title">Private AI</span>
                <span className="ftux-card-text">
                  {privateAi
                    ? "Enabled. Your tools are connected to NEAR AI's private infrastructure from the first session."
                    : "Enable to connect Private Inference from Near.AI. Your tools and session data will be kept private from the first connection."}
                </span>
              </span>
              <GlassSwitch
                checked={privateAi}
                label="Private AI"
                onToggle={onTogglePrivateAi}
              />
            </div>
          </div>
        )}

        {error ? (
          <p className="ftux-well" role="alert" style={{ color: "#ff8a80" }}>
            {error}
          </p>
        ) : null}
      </div>
      <div className="ftux-footer">
        <button
          type="button"
          className="ftux-btn ftux-btn-primary"
          disabled={submitting}
          onClick={onStart}
        >
          {submitting ? <Spinner /> : null}
          Start sharing
        </button>
      </div>
    </>
  );
}
