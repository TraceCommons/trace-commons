import { useId, useState } from "react";
import {
  ButtonPrimary,
  Card,
  Checkbox,
  Expander,
  Picker,
  type PickerOption,
  TertiaryLink,
} from "../../../design-system";
import {
  formatDuration,
  formatSessionDate,
  groupState,
  pastSessionSummary,
  REPO_RULE_LABELS,
  type RepoRule,
  type RepoSelection,
} from "../ftux-model";
import type { PastSession, RepoCandidate } from "../types";
import { ScreenBody, ScreenFooter, ScreenTitle } from "./ftux-frame";
import { Spinner } from "./icons";

const RULE_DOTS: Record<RepoRule, string> = {
  ask: "var(--tc-status-ask)",
  auto: "var(--tc-status-on)",
  never: "var(--tc-status-off)",
};

const RULE_OPTIONS: PickerOption<RepoRule>[] = (
  ["ask", "auto", "never"] as const
).map((rule) => ({
  value: rule,
  label: REPO_RULE_LABELS[rule],
  dot: RULE_DOTS[rule],
}));

const COLLAPSED_SESSIONS = 2;

function sessionLabel(session: PastSession) {
  return [
    formatSessionDate(session.date),
    session.title,
    session.durationMinutes === null
      ? null
      : formatDuration(session.durationMinutes),
  ]
    .filter(Boolean)
    .join(" · ");
}

function PastSessionFolder({
  candidate,
  selection,
  defaultOpen,
  onToggleAll,
  onToggleSession,
}: {
  candidate: RepoCandidate;
  selection: RepoSelection;
  defaultOpen: boolean;
  onToggleAll: () => void;
  onToggleSession: (index: number) => void;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const [showAll, setShowAll] = useState(false);
  const listId = useId();
  const count = candidate.sessions.length;
  const on = selection.selected.filter(Boolean).length;

  if (selection.rule === "never") {
    return (
      <div className="tc-check-row ftux-muted-row">
        <Checkbox
          checked={false}
          disabled
          label={`${candidate.folder}: rule is Never`}
        />
        <span className="tc-mono ftux-grow">{candidate.folder}</span>
        <span className="tc-caption">{count} · rule is Never</span>
      </div>
    );
  }

  const state = groupState(selection.selected);
  const visible = showAll
    ? candidate.sessions
    : candidate.sessions.slice(0, COLLAPSED_SESSIONS);
  return (
    <div className="tc-stack tc-stack--tight">
      <div className="tc-check-row">
        <Checkbox
          checked={state === "all"}
          indeterminate={state === "some"}
          label={`Include every past session in ${candidate.folder}`}
          onChange={onToggleAll}
        />
        <span className="ftux-grow">
          <Expander
            open={open}
            onToggle={() => setOpen(!open)}
            controls={listId}
          >
            <span className="tc-mono">{candidate.folder}</span>
          </Expander>
        </span>
        <span className="tc-caption tc-text-tertiary">
          {on} of {count}
        </span>
      </div>
      {open ? (
        <div id={listId} className="tc-stack tc-stack--tight ftux-indent">
          {visible.map((session, index) => (
            <div key={session.id} className="tc-check-row">
              <Checkbox
                checked={selection.selected[index] === true}
                label={sessionLabel(session)}
                onChange={() => onToggleSession(index)}
              />
              <span className="tc-label">{sessionLabel(session)}</span>
            </div>
          ))}
          {count > COLLAPSED_SESSIONS ? (
            <TertiaryLink
              className="ftux-self-start"
              onClick={() => setShowAll(!showAll)}
            >
              {showAll ? "Show fewer" : `Show all ${count}`}
            </TertiaryLink>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

// Custom setup: W-5.
export function RulesScreen({
  candidates,
  selections,
  sourceName,
  onRule,
  onToggleAll,
  onToggleSession,
  onContinue,
}: {
  candidates: RepoCandidate[] | null;
  selections: RepoSelection[];
  sourceName: string;
  onRule: (folder: string, rule: RepoRule) => void;
  onToggleAll: (folder: string) => void;
  onToggleSession: (folder: string, index: number) => void;
  onContinue: () => void;
}) {
  const summary = pastSessionSummary(selections);
  const byFolder = new Map(selections.map((s) => [s.folder, s]));
  return (
    <>
      <ScreenTitle light="Set your " bold="rules and permissions." />
      <ScreenBody>
        {candidates === null ? (
          <p className="tc-status tc-text-secondary m-0" role="status">
            <Spinner /> Reading repos from your sessions…
          </p>
        ) : candidates.length === 0 ? (
          <Card quiet className="tc-text-secondary">
            No repos to set rules for yet. Rules appear for repos found in the
            sessions of a tool you watch.
          </Card>
        ) : (
          <>
            <Card className="tc-stack tc-stack--tight">
              <span className="tc-eyebrow">
                Repos found in {sourceName} sessions
              </span>
              {candidates.map((candidate) => {
                const selection = byFolder.get(candidate.folder);
                if (!selection) return null;
                return (
                  <div
                    key={candidate.folder}
                    className="ftux-row ftux-row--between tc-hairline-top ftux-repo-row"
                  >
                    <span className="tc-stack ftux-gap-0 ftux-min-0">
                      <span className="tc-mono tc-text-primary ftux-ellipsis">
                        {candidate.folder}
                      </span>
                      <span className="tc-caption tc-text-tertiary">
                        {candidate.sessions.length} sessions
                        {candidate.note ? ` · ${candidate.note}` : ""}
                      </span>
                    </span>
                    <Picker
                      label={`Rule for ${candidate.folder}`}
                      value={selection.rule}
                      options={RULE_OPTIONS}
                      onChange={(rule) => onRule(candidate.folder, rule)}
                    />
                  </div>
                );
              })}
            </Card>
            <Card className="tc-stack">
              <div className="ftux-row ftux-row--between">
                <span className="tc-body-strong">Past sessions, by folder</span>
                <span className="tc-caption tc-text-tertiary">
                  {summary.label}
                </span>
              </div>
              {candidates.map((candidate, index) => {
                const selection = byFolder.get(candidate.folder);
                if (!selection) return null;
                return (
                  <PastSessionFolder
                    key={candidate.folder}
                    candidate={candidate}
                    selection={selection}
                    defaultOpen={index === 0}
                    onToggleAll={() => onToggleAll(candidate.folder)}
                    onToggleSession={(i) =>
                      onToggleSession(candidate.folder, i)
                    }
                  />
                );
              })}
            </Card>
          </>
        )}
      </ScreenBody>
      <ScreenFooter>
        <ButtonPrimary disabled={candidates === null} onClick={onContinue}>
          Continue
        </ButtonPrimary>
      </ScreenFooter>
    </>
  );
}
