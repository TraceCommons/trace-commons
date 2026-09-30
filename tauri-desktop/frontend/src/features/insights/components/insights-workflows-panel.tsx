import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useController, useForm } from "react-hook-form";
import { type WorkflowFormValues, workflowFormSchema } from "../forms";
import type { Insight } from "../types";
import type {
  EpisodeDetail,
  EpisodeListEntry,
  QuestionCardResult,
} from "../workflows";

type WorkflowApi = {
  episodes: EpisodeListEntry[];
  detail: EpisodeDetail | null;
  cards: QuestionCardResult | null;
  state: "loading" | "ready" | "busy" | "error";
  open: (id: string) => void;
  close: () => void;
  create: (ids: string[]) => Promise<boolean>;
  calculateCards: (snapshotIds: string[], episodeIds: string[]) => void;
  annotate: (category: string, outcome: string) => Promise<boolean>;
};

function valueLabel(
  value: { type: string; value: number } | null,
  missing: string | null,
) {
  if (!value) return missing?.replaceAll("_", " ") ?? "Unavailable";
  if (value.type === "unix_milliseconds")
    return new Date(value.value).toLocaleString();
  if (value.type === "milliseconds") return `${value.value} ms`;
  return value.value.toLocaleString();
}

export function InsightsWorkflowsPanel({
  snapshots,
  workflow,
}: {
  snapshots: Insight[];
  workflow: WorkflowApi;
}) {
  const form = useForm<WorkflowFormValues>({
    resolver: zodResolver(workflowFormSchema),
    defaultValues: {
      snapshotSelection: [],
      episodeSelection: [],
      category: "unknown",
      outcome: "unknown",
    },
  });
  const snapshotSelection = useController({
    control: form.control,
    name: "snapshotSelection",
  });
  const episodeSelection = useController({
    control: form.control,
    name: "episodeSelection",
  });
  const workflowIdentity = workflow.detail?.episode.id ?? "list";
  useEffect(() => {
    if (workflowIdentity && !form.formState.isDirty) {
      form.reset({
        snapshotSelection: [],
        episodeSelection: [],
        category: "unknown",
        outcome: "unknown",
      });
    }
  }, [form, workflowIdentity]);
  const busy = workflow.state === "busy" || workflow.state === "loading";
  const toggle = (
    field: { field: { value: string[]; onChange: (value: string[]) => void } },
    id: string,
  ) => {
    const values = field.field.value;
    field.field.onChange(
      values.includes(id)
        ? values.filter((value) => value !== id)
        : [...values, id],
    );
  };

  return (
    <>
      <section className="tc-card mb-2.5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <span className="mb-1.5 block tc-eyebrow">
              EPISODES
            </span>
            <h2>Group saved snapshots</h2>
          </div>
          <span className="tc-chip tc-chip--glass self-start">
            {workflow.episodes.length}
          </span>
        </div>
        <p>
          Select whole saved snapshots. Groups can overlap; they do not
          establish task boundaries or verify outcomes.
        </p>
        <div className="my-3 grid gap-px">
          {snapshots.length === 0 ? (
            <p className="mt-3 mb-1 tc-body tc-text-tertiary">
              Save at least one snapshot before creating an episode.
            </p>
          ) : (
            snapshots.map((snapshot) => (
              <label
                className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal"
                key={snapshot.id}
              >
                <Checkbox
                  checked={snapshotSelection.field.value.includes(snapshot.id)}
                  onCheckedChange={() => toggle(snapshotSelection, snapshot.id)}
                  disabled={busy}
                />
                <span>
                  <strong>{snapshot.source_format}</strong>
                  <small>{snapshot.id}</small>
                </span>
              </label>
            ))
          )}
        </div>
        <Button
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={() =>
            void form.handleSubmit(async (values) => {
              if (await workflow.create(values.snapshotSelection))
                form.setValue("snapshotSelection", [], { shouldDirty: false });
            })()
          }
          disabled={busy || snapshotSelection.field.value.length === 0}
        >
          Create episode
        </Button>
        {workflow.detail ? (
          <div className="mt-4 p-[26px]">
            <div className="flex items-start justify-between gap-3">
              <div>
                <span className="mb-1.5 block tc-eyebrow">
                  EPISODE DETAIL
                </span>
                <h3>{workflow.detail.episode.id}</h3>
              </div>
              <Button
                className="tc-link"
                type="button"
                onClick={workflow.close}
              >
                Back to episodes
              </Button>
            </div>
            <p className="m-0 tc-caption tc-text-tertiary">
              Resolved {new Date(workflow.detail.resolved_at).toLocaleString()}.
              Membership revision {workflow.detail.episode.membership_revision}.
              Overlap is informational.
            </p>
            <div className="mt-3 grid gap-px">
              {workflow.detail.members.map((member) => (
                <div
                  className="grid grid-cols-[38px_minmax(0,1fr)_auto_auto] items-center gap-3.5 border-b border-border py-3.5 max-[860px]:grid-cols-[38px_minmax(0,1fr)_auto]"
                  key={member.id}
                >
                  <span className="tc-tool-tile tc-tool-tile--lg">
                    {member.source_format.slice(0, 1).toUpperCase()}
                  </span>
                  <span className="grid min-w-0 gap-1">
                    <strong>{member.source_format}</strong>
                    <span>{member.id}</span>
                  </span>
                  <span className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
                    <strong>
                      {member.manual_annotation?.outcome ?? "Unassessed"}
                    </strong>
                    <span>Member snapshot</span>
                  </span>
                </div>
              ))}
            </div>
            <div className="my-3 flex gap-3">
              <label>
                Category
                <NativeSelect {...form.register("category")} disabled={busy}>
                  <option value="unknown">Unknown</option>
                  <option value="refactor">Refactor</option>
                  <option value="tests">Tests</option>
                  <option value="docs">Docs</option>
                  <option value="debugging">Debugging</option>
                  <option value="other">Other</option>
                </NativeSelect>
              </label>
              <label>
                Outcome
                <NativeSelect {...form.register("outcome")} disabled={busy}>
                  <option value="unknown">Unknown</option>
                  <option value="accepted">Accepted</option>
                  <option value="partial">Partial</option>
                  <option value="rejected">Rejected</option>
                </NativeSelect>
              </label>
            </div>
            <Button
              className="tc-btn tc-btn--glass"
              type="button"
              onClick={() =>
                void form.handleSubmit(async (values) => {
                  if (await workflow.annotate(values.category, values.outcome))
                    form.reset(values);
                })()
              }
              disabled={busy}
            >
              Save episode assessment
            </Button>
            <p className="m-0 tc-caption tc-text-tertiary">
              Assessment is user-reported. It does not turn member evidence into
              a verified result.
            </p>
          </div>
        ) : (
          <div className="mt-3 grid gap-px">
            {workflow.episodes.length === 0 ? (
              <p className="mt-3 mb-1 tc-body tc-text-tertiary">
                No saved episodes.
              </p>
            ) : (
              workflow.episodes.map((entry) => (
                <Button
                  className="grid w-full grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-lg border-0 bg-transparent px-1.5 py-2 text-left text-inherit hover:bg-white/5 tc-hairline-bottom"
                  type="button"
                  key={entry.episode.id}
                  onClick={() => workflow.open(entry.episode.id)}
                  disabled={busy}
                >
                  <span className="tc-tool-tile tc-tool-tile--lg tc-tool-tile--folder">
                    E
                  </span>
                  <span className="grid min-w-0 gap-1">
                    <strong>
                      {entry.episode.members.length} whole snapshots
                    </strong>
                    <span>{entry.episode.id}</span>
                  </span>
                  <span className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
                    <strong>
                      {entry.episode.manual_assessment?.outcome ?? "Unassessed"}
                    </strong>
                    <span>
                      {entry.overlapping_episode_ids.length
                        ? `${entry.overlapping_episode_ids.length} overlaps`
                        : "No overlap"}
                    </span>
                  </span>
                </Button>
              ))
            )}
          </div>
        )}
      </section>
      <section className="tc-card mb-2.5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <span className="mb-1.5 block tc-eyebrow">
              QUESTION CARDS
            </span>
            <h2>Ask deterministic local questions</h2>
          </div>
          <span className="tc-chip tc-chip--glass self-start">
            LOCAL
          </span>
        </div>
        <p>
          Cards summarize selected saved evidence. They do not infer active
          time, independence, quality, or model advantage.
        </p>
        <div className="my-3 grid gap-px">
          {workflow.episodes.map((entry) => (
            <label
              className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal"
              key={entry.episode.id}
            >
              <Checkbox
                checked={episodeSelection.field.value.includes(
                  entry.episode.id,
                )}
                onCheckedChange={() => toggle(episodeSelection, entry.episode.id)}
                disabled={busy}
              />
              <span>
                <strong>
                  Episode · {entry.episode.members.length} snapshots
                </strong>
                <small>{entry.episode.id}</small>
              </span>
            </label>
          ))}
        </div>
        <Button
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={() =>
            void form.handleSubmit((values) =>
              workflow.calculateCards(
                values.snapshotSelection,
                values.episodeSelection,
              ),
            )()
          }
          disabled={
            busy ||
            (snapshotSelection.field.value.length === 0 &&
              episodeSelection.field.value.length === 0)
          }
        >
          Calculate cards
        </Button>
        {workflow.cards && (
          <div className="mt-[18px] grid grid-cols-2 gap-2.5 max-[860px]:grid-cols-1">
            {workflow.cards.cards.map((card) => (
              <article
                className="tc-card tc-card--quiet grid gap-[6px] min-h-[150px] gap-2.5"
                key={card.question}
              >
                <div className="flex items-start justify-between gap-3">
                  <strong>{card.question.replaceAll("_", " ")}</strong>
                  <span className="tc-chip self-start">
                    {card.state}
                  </span>
                </div>
                {card.rows.map((row) => (
                  <div
                    className="flex justify-between gap-3 border-t border-border pt-2 text-[11px] text-muted-foreground"
                    key={row.id}
                  >
                    <span>{row.label ?? row.id.replaceAll("_", " ")}</span>
                    <b>{valueLabel(row.value, row.missing_reason)}</b>
                  </div>
                ))}
                <small className="m-0 tc-caption tc-text-tertiary">
                  Coverage:{" "}
                  {card.coverage
                    .map(
                      (item) =>
                        `${item.unit.replaceAll("_", " ")} ${item.observed}/${item.eligible}`,
                    )
                    .join(" · ") || "none"}
                </small>
              </article>
            ))}
          </div>
        )}
        {workflow.cards && (
          <p className="m-0 tc-caption tc-text-tertiary">
            Input digest {workflow.cards.input_digest}. Provider{" "}
            {workflow.cards.provider.id}, rubric{" "}
            {workflow.cards.provider.rubric_version}.
          </p>
        )}
      </section>
    </>
  );
}
