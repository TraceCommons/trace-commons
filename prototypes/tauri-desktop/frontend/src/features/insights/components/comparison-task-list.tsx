import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { UseFormReturn } from "react-hook-form";
import type { ComparisonTaskDetail } from "../comparisons";
import type { ComparisonTaskFormValues } from "../forms";
import type { EpisodeListEntry } from "../workflows";
import type { SelectionField } from "./comparison-task-types";

export function ComparisonTaskList({
  episodes,
  tasks,
  form,
  episodeSelection,
  busy,
  onToggle,
  onCreate,
  onOpen,
}: {
  episodes: EpisodeListEntry[];
  tasks: ComparisonTaskDetail[];
  form: UseFormReturn<ComparisonTaskFormValues>;
  episodeSelection: SelectionField;
  busy: boolean;
  onToggle: (field: SelectionField, id: string) => void;
  onCreate: (ids: string[]) => undefined | Promise<boolean | undefined>;
  onOpen: (id: string) => void;
}) {
  return (
    <>
      <div className="my-[18px] grid gap-px border-t border-border">
        {episodes.length === 0 ? (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            Create an episode first.
          </p>
        ) : (
          episodes.map((entry) => (
            <label
              className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
              key={entry.episode.id}
            >
              <Checkbox
                checked={episodeSelection.value.includes(entry.episode.id)}
                onCheckedChange={() => onToggle(episodeSelection, entry.episode.id)}
                disabled={busy}
              />
              <span>
                <strong>{entry.episode.members.length} snapshots</strong>
                <small>{entry.episode.id}</small>
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
            const accepted = await onCreate(values.episodeSelection);
            if (accepted !== false)
              form.setValue("episodeSelection", [], { shouldDirty: false });
          })()
        }
        disabled={busy || episodeSelection.value.length === 0}
      >
        Create comparison task
      </Button>
      <div className="mt-[22px] grid gap-px border-t border-border">
        {tasks.map((item) => (
          <Button
            className="grid w-full grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-0 border-b border-border bg-transparent py-3.5 text-left hover:bg-muted"
            type="button"
            key={item.task.id}
            onClick={() => onOpen(item.task.id)}
            disabled={busy}
          >
            <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground">
              C
            </span>
            <span className="grid min-w-0 gap-1">
              <strong>
                {item.task.context?.task_date ?? "Context incomplete"}
              </strong>
              <span>{item.task.id}</span>
            </span>
            <span className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
              <strong>{item.task.outcome?.value ?? "Unassessed"}</strong>
              <span>
                {item.stale_reasons.length ? "Review required" : "Current"}
              </span>
            </span>
          </Button>
        ))}
      </div>
    </>
  );
}
