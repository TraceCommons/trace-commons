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
      <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
        <div className="flex items-start justify-between gap-[18px]">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              EPISODES
            </span>
            <h2>Group saved snapshots</h2>
          </div>
          <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
            {workflow.episodes.length}
          </span>
        </div>
        <p>
          Select whole saved snapshots. Groups can overlap; they do not
          establish task boundaries or verify outcomes.
        </p>
        <div className="my-[18px] grid gap-px border-t border-border">
          {snapshots.length === 0 ? (
            <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
              Save at least one snapshot before creating an episode.
            </p>
          ) : (
            snapshots.map((snapshot) => (
              <label
                className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
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
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
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
            <div className="flex items-start justify-between gap-[18px]">
              <div>
                <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                  EPISODE DETAIL
                </span>
                <h3>{workflow.detail.episode.id}</h3>
              </div>
              <Button
                className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
                type="button"
                onClick={workflow.close}
              >
                Back to episodes
              </Button>
            </div>
            <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
              Resolved {new Date(workflow.detail.resolved_at).toLocaleString()}.
              Membership revision {workflow.detail.episode.membership_revision}.
              Overlap is informational.
            </p>
            <div className="mt-[22px] grid gap-px border-t border-border">
              {workflow.detail.members.map((member) => (
                <div
                  className="grid grid-cols-[38px_minmax(0,1fr)_auto_auto] items-center gap-3.5 border-b border-border py-3.5 max-[860px]:grid-cols-[38px_minmax(0,1fr)_auto]"
                  key={member.id}
                >
                  <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground">
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
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
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
            <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
              Assessment is user-reported. It does not turn member evidence into
              a verified result.
            </p>
          </div>
        ) : (
          <div className="mt-[22px] grid gap-px border-t border-border">
            {workflow.episodes.length === 0 ? (
              <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
                No saved episodes.
              </p>
            ) : (
              workflow.episodes.map((entry) => (
                <Button
                  className="grid w-full grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-0 border-b border-border bg-transparent py-3.5 text-left hover:bg-muted"
                  type="button"
                  key={entry.episode.id}
                  onClick={() => workflow.open(entry.episode.id)}
                  disabled={busy}
                >
                  <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground bg-blue">
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
      <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
        <div className="flex items-start justify-between gap-[18px]">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              QUESTION CARDS
            </span>
            <h2>Ask deterministic local questions</h2>
          </div>
          <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
            LOCAL
          </span>
        </div>
        <p>
          Cards summarize selected saved evidence. They do not infer active
          time, independence, quality, or model advantage.
        </p>
        <div className="my-[18px] grid gap-px border-t border-border">
          {workflow.episodes.map((entry) => (
            <label
              className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
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
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
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
                className="grid gap-[6px] rounded-[10px] border border-border bg-muted p-3.5 min-h-[150px] gap-2.5"
                key={card.question}
              >
                <div className="flex items-start justify-between gap-[18px]">
                  <strong>{card.question.replaceAll("_", " ")}</strong>
                  <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start bg-muted text-muted-foreground">
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
                <small className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
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
          <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
            Input digest {workflow.cards.input_digest}. Provider{" "}
            {workflow.cards.provider.id}, rubric{" "}
            {workflow.cards.provider.rubric_version}.
          </p>
        )}
      </section>
    </>
  );
}
