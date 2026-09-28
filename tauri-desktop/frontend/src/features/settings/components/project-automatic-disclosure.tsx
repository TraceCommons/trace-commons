import { scrubDisclosureLines } from "../../../lib/tauri/automatic-grant-copy";
import { useProjectAutomaticCopy } from "../../../lib/tauri/use-contributor-copy";

// K6: what an armed project's sessions have had removed, in the words the
// contributor core chose for this project from its own sessions'
// certificates. Model-scrub wording appears only once every session it has
// sent unattended since it was armed carried a certified full pipeline; the
// deterministic-only wording otherwise. This component renders the core's
// sentences and never chooses between them.
export function ProjectAutomaticDisclosure({
  projectId,
  disclosure,
}: {
  projectId: string;
  disclosure: string | undefined;
}) {
  const query = useProjectAutomaticCopy(projectId, disclosure);
  const copy = query.data;
  if (!copy) {
    return query.isError ? (
      <small role="alert">What is removed from this project could not be loaded.</small>
    ) : null;
  }
  return (
    <div className="mt-2 grid gap-1.5 text-[11px] leading-[1.55] text-muted-foreground">
      {scrubDisclosureLines(copy).map((line) => (
        <p className="m-0" key={line}>
          {line}
        </p>
      ))}
    </div>
  );
}
