import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { groupState, OPTIONAL_USES, optionalUsesLabel } from "../ftux-model";
import type { SharingMode } from "../types";
import {
  ChevronIcon,
  GlassCheckbox,
  PillSelect,
  ScreenTitle,
  type SelectOption,
  Spinner,
} from "./glass";
import { PrivateAiCard } from "./private-ai-card";

const SHARING_OPTIONS: SelectOption<SharingMode>[] = [
  { value: "auto", label: "Share automatically", tone: "green" },
  { value: "ask", label: "Ask me each time", tone: "yellow" },
];

// W-3 (Quick setup) and W-6 (Custom setup, which adds the Private AI
// switch). Uses are the data-use scope (R7), so they are answered before the
// sharing decision on the same screen: a grant covers the scopes chosen when
// it is given, and widening them later voids it.
export function UsesScreen({
  baseUse,
  optionalUses,
  listHandle,
  sharing,
  privateAi,
  submitting,
  error,
  onToggleBaseUse,
  onToggleUses,
  onToggleUse,
  onToggleHandle,
  onSharing,
  onTogglePrivateAi,
  onStart,
}: {
  // The floor scope: required, but ticked by hand like the rest (R7).
  baseUse: boolean;
  optionalUses: boolean[];
  listHandle: boolean;
  sharing: SharingMode;
  // Null hides the Private AI card (Quick setup).
  privateAi: boolean | null;
  submitting: boolean;
  error: string | null;
  onToggleBaseUse: () => void;
  onToggleUses: () => void;
  onToggleUse: (index: number) => void;
  onToggleHandle: () => void;
  onSharing: (mode: SharingMode) => void;
  onTogglePrivateAi: () => void;
  onStart: () => void;
}) {
  const usesState = groupState(optionalUses);
  // The sharing words are the core's: which scrub runs, and what leaves the
  // machine, depend on the configuration, so this screen never states them.
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  const copy = grantCopy.data;
  const canStart = baseUse && copy !== undefined && !submitting;
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
              checked={baseUse}
              label="Finding bugs and measuring agents, required"
              onToggle={onToggleBaseUse}
            />
            <span style={{ flex: 1 }}>
              <strong>Finding bugs and measuring agents</strong>
              <span className="ftux-always-on">required</span>
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
                {copy
                  ? sharing === "auto"
                    ? `${copy.path_automatic} ${copy.scrub.scope} ${copy.scrub.limit}`
                    : copy.path_ask_first
                  : grantCopy.isError
                    ? "Sharing copy unavailable. Starting is disabled."
                    : "Loading sharing copy…"}
              </span>
            </span>
            <PillSelect
              label="Sharing"
              disabled={!copy}
              value={sharing}
              options={SHARING_OPTIONS}
              onChange={onSharing}
            />
          </div>
        </div>

        {privateAi === null ? null : (
          <PrivateAiCard checked={privateAi} onToggle={onTogglePrivateAi} />
        )}

        {error ? (
          <p className="ftux-well" role="alert" style={{ color: "#ff8a80" }}>
            {error}
          </p>
        ) : null}
      </div>
      <div className={`ftux-footer${baseUse ? "" : " ftux-footer-split"}`}>
        {baseUse ? null : (
          <span className="ftux-card-text ftux-muted" id="ftux-scope-note">
            Tick the first use to contribute. Without it nothing is shared.
          </span>
        )}
        <button
          type="button"
          className="ftux-btn ftux-btn-primary"
          disabled={!canStart}
          aria-describedby={baseUse ? undefined : "ftux-scope-note"}
          onClick={onStart}
        >
          {submitting ? <Spinner /> : null}
          Start sharing
        </button>
      </div>
    </>
  );
}
