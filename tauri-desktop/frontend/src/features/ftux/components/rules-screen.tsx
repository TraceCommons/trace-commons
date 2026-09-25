import { useState } from "react";
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
import {
  ChevronIcon,
  GlassCheckbox,
  PillSelect,
  ScreenTitle,
  type SelectOption,
  Spinner,
} from "./glass";

const RULE_TONES: Record<RepoRule, SelectOption<RepoRule>["tone"]> = {
  ask: "yellow",
  auto: "green",
  never: "grey",
};

const RULE_OPTIONS: SelectOption<RepoRule>[] = (
  ["ask", "auto", "never"] as const
).map((rule) => ({
  value: rule,
  label: REPO_RULE_LABELS[rule],
  tone: RULE_TONES[rule],
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
  const [showAll, setShowAll] = useState(false);
  const count = candidate.sessions.length;
  const on = selection.selected.filter(Boolean).length;

  if (selection.rule === "never") {
    return (
      <div
        className="ftux-check-row ftux-muted"
        style={{ alignItems: "center", opacity: 0.7 }}
      >
        <GlassCheckbox
          checked={false}
          disabled
          label={`${candidate.folder}: rule is Never`}
        />
        <span className="ftux-mono" style={{ flex: 1 }}>
          {candidate.folder}
        </span>
        <span>{count} · rule is Never</span>
      </div>
    );
  }

  const state = groupState(selection.selected);
  const visible = showAll
    ? candidate.sessions
    : candidate.sessions.slice(0, COLLAPSED_SESSIONS);
  return (
    <div className="ftux-check-row">
      <GlassCheckbox
        checked={state === "all" ? true : state === "some" ? "mixed" : false}
        label={`Include every past session in ${candidate.folder}`}
        onToggle={onToggleAll}
      />
      <details className="ftux-disclosure" open={defaultOpen}>
        <summary>
          <span className="ftux-mono" style={{ flex: 1, color: "#f2f2f4" }}>
            {candidate.folder}
          </span>
          <span className="ftux-muted">
            {on} of {count}
          </span>
          <ChevronIcon size={12} />
        </summary>
        <div className="ftux-disclosure-body">
          {visible.map((session, index) => (
            <div key={session.id} className="ftux-session-row">
              <GlassCheckbox
                checked={selection.selected[index] === true}
                label={sessionLabel(session)}
                onToggle={() => onToggleSession(index)}
              />
              <span style={{ flex: 1 }}>{sessionLabel(session)}</span>
            </div>
          ))}
          {count > COLLAPSED_SESSIONS ? (
            <button
              type="button"
              className="ftux-link"
              style={{ alignSelf: "flex-start", marginLeft: 25 }}
              onClick={() => setShowAll(!showAll)}
            >
              {showAll ? "Show fewer" : `Show all ${count}`}
            </button>
          ) : null}
        </div>
      </details>
    </div>
  );
}

// Customize and tailor: W-5.
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
      <div className="ftux-scroll">
        {candidates === null ? (
          <p className="ftux-status" role="status">
            <Spinner /> Reading repos from your sessions…
          </p>
        ) : candidates.length === 0 ? (
          <p className="ftux-well">
            No repos yet. Rules appear here once a watched tool records a
            session.
          </p>
        ) : (
          <>
            <div className="ftux-card">
              <span className="ftux-section-label">
                Repos found in {sourceName} sessions
              </span>
              {candidates.map((candidate) => {
                const selection = byFolder.get(candidate.folder);
                if (!selection) return null;
                return (
                  <div key={candidate.folder} className="ftux-repo-row">
                    <span className="ftux-repo-name">
                      <span title={candidate.folder}>{candidate.folder}</span>
                      <span className="ftux-tool-meta">
                        {candidate.sessions.length} sessions
                        {candidate.note ? ` · ${candidate.note}` : ""}
                      </span>
                    </span>
                    <PillSelect
                      label={`Rule for ${candidate.folder}`}
                      value={selection.rule}
                      options={RULE_OPTIONS}
                      onChange={(rule) => onRule(candidate.folder, rule)}
                    />
                  </div>
                );
              })}
            </div>
            <div className="ftux-card" style={{ gap: 10 }}>
              <div className="ftux-card-row">
                <span className="ftux-card-title">
                  Past sessions, by folder
                </span>
                <span className="ftux-card-text ftux-muted">
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
            </div>
          </>
        )}
      </div>
      <div className="ftux-footer">
        <button
          type="button"
          className="ftux-btn ftux-btn-primary"
          disabled={candidates === null}
          onClick={onContinue}
        >
          Continue
        </button>
      </div>
    </>
  );
}
