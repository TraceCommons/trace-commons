import { useId, useState } from "react";
import {
  ButtonPrimary,
  Card,
  Checkbox,
  Expander,
  Picker,
  type PickerOption,
} from "../../../design-system";
import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { groupState, OPTIONAL_USES, optionalUsesLabel } from "../ftux-model";
import type { SharingMode } from "../types";
import {
  ScreenBody,
  ScreenFooter,
  ScreenTitle,
  StatusLine,
} from "./ftux-frame";
import { Spinner } from "./icons";
import { PrivateAiCard } from "./private-ai-card";

const SHARING_OPTIONS: PickerOption<SharingMode>[] = [
  { value: "auto", label: "Share automatically", dot: "var(--tc-status-on)" },
  { value: "ask", label: "Ask me each time", dot: "var(--tc-status-ask)" },
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
  // The sharing words are the core's: which scrub runs, and what leaves the
  // machine, depend on the configuration, so this screen never states them.
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  const copy = grantCopy.data;
  const canStart = baseUse && copy !== undefined && !submitting;
  return (
    <>
      <ScreenTitle light="How your data is " bold="used & permissioned." />
      <ScreenBody>
        <UsesCard
          baseUse={baseUse}
          optionalUses={optionalUses}
          listHandle={listHandle}
          onToggleBaseUse={onToggleBaseUse}
          onToggleUses={onToggleUses}
          onToggleUse={onToggleUse}
          onToggleHandle={onToggleHandle}
        />
        <Card>
          <div className="ftux-row ftux-row--between ftux-row--top">
            <span className="tc-stack ftux-gap-2">
              <span className="tc-body-strong">Sharing</span>
              <span className="tc-label tc-text-secondary">
                {copy
                  ? sharing === "auto"
                    ? `${copy.path_automatic} ${copy.scrub.scope} ${copy.scrub.limit}`
                    : copy.path_ask_first
                  : grantCopy.isError
                    ? "Sharing copy unavailable. Starting is disabled."
                    : "Loading sharing copy…"}
              </span>
            </span>
            <Picker
              label="Sharing"
              value={sharing}
              options={SHARING_OPTIONS}
              disabled={!copy}
              onChange={onSharing}
            />
          </div>
        </Card>
        {privateAi === null ? null : (
          <PrivateAiCard checked={privateAi} onToggle={onTogglePrivateAi} />
        )}
        {error ? <StatusLine tone="error">{error}</StatusLine> : null}
      </ScreenBody>
      <ScreenFooter
        noteId="ftux-scope-note"
        note={
          baseUse
            ? undefined
            : "Tick the first use to contribute. Without it nothing is shared."
        }
      >
        <ButtonPrimary
          disabled={!canStart}
          aria-describedby={baseUse ? undefined : "ftux-scope-note"}
          onClick={onStart}
        >
          {submitting ? <Spinner /> : null}
          Start sharing
        </ButtonPrimary>
      </ScreenFooter>
    </>
  );
}

function UsesCard({
  baseUse,
  optionalUses,
  listHandle,
  onToggleBaseUse,
  onToggleUses,
  onToggleUse,
  onToggleHandle,
}: {
  baseUse: boolean;
  optionalUses: boolean[];
  listHandle: boolean;
  onToggleBaseUse: () => void;
  onToggleUses: () => void;
  onToggleUse: (index: number) => void;
  onToggleHandle: () => void;
}) {
  const [open, setOpen] = useState(false);
  const listId = useId();
  const usesState = groupState(optionalUses);
  return (
    <Card className="tc-stack tc-stack--tight">
      <span className="tc-eyebrow">How your traces may be used</span>
      <div className="tc-check-row">
        <Checkbox
          checked={baseUse}
          label="Finding bugs and measuring agents, required"
          onChange={onToggleBaseUse}
        />
        <span>
          <span className="tc-body-strong">
            Finding bugs and measuring agents
          </span>{" "}
          <span className="tc-mono tc-text-on">required</span>
          <span className="tc-caption tc-text-tertiary ftux-block">
            Researchers read traces to see where coding agents fail and to score
            agents against each other.
          </span>
        </span>
      </div>
      <div className="tc-check-row">
        <Checkbox
          checked={usesState === "all"}
          indeterminate={usesState === "some"}
          label="All optional uses"
          onChange={onToggleUses}
        />
        <Expander open={open} onToggle={() => setOpen(!open)} controls={listId}>
          <span className="tc-body-strong">
            {optionalUsesLabel(optionalUses)}
          </span>
        </Expander>
      </div>
      {open ? (
        <div id={listId} className="tc-stack tc-stack--tight ftux-indent">
          {OPTIONAL_USES.map((use, index) => (
            <div key={use} className="tc-check-row">
              <Checkbox
                checked={optionalUses[index] === true}
                label={use}
                onChange={() => onToggleUse(index)}
              />
              <span className="tc-body-strong">{use}</span>
            </div>
          ))}
        </div>
      ) : null}
      <div className="tc-check-row">
        <Checkbox
          checked={listHandle}
          label="List my handle publicly as a contributor"
          onChange={onToggleHandle}
        />
        <span>
          <span className="tc-body-strong">
            List my handle publicly as a contributor
          </span>
          <span className="tc-caption tc-text-tertiary ftux-block">
            Credit only. It does not change how any trace is used.
          </span>
        </span>
      </div>
    </Card>
  );
}
