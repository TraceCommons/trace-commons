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
      <div className="my-3 grid gap-px">
        {episodes.length === 0 ? (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
            Create an episode first.
          </p>
        ) : (
          episodes.map((entry) => (
            <label
              className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal"
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
        className="tc-btn tc-btn--glass"
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
      <div className="mt-3 grid gap-px">
        {tasks.map((item) => (
          <Button
            className="grid w-full grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-lg border-0 bg-transparent px-1.5 py-2 text-left text-inherit hover:bg-white/5 tc-hairline-bottom"
            type="button"
            key={item.task.id}
            onClick={() => onOpen(item.task.id)}
            disabled={busy}
          >
            <span className="tc-tool-tile tc-tool-tile--lg">
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
