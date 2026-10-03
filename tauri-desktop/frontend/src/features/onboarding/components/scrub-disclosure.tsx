import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { ButtonPrimary, GlassButton, Skeleton } from "@/design-system";

type ScrubDisclosureProps = {
  open: boolean;
  names: string[];
  state: "idle" | "loading" | "ready" | "error";
  error: string | null;
  onRetry: () => void;
  onClose: () => void;
};

export function ScrubDisclosure({
  open,
  names,
  state,
  error,
  onRetry,
  onClose,
}: ScrubDisclosureProps) {
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose();
      }}
      title="What gets removed?"
      description="Local scrubbing checks named secret categories before contribution. Detector patterns stay private."
      footer={
        <ButtonPrimary size="sm" onClick={onClose}>
          Close
        </ButtonPrimary>
      }
    >
      <div className="grid gap-4 pb-4">
        <p className="text-sm text-tc-secondary">
          Before anything leaves this machine, local scrubbing looks for these
          named secret categories. Names are shown; detector patterns stay
          private.
        </p>
        {state === "loading" && (
          <div className="grid gap-2" role="status" aria-label="Reading detector list">
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-4 w-1/2" />
            <Skeleton className="h-4 w-3/4" />
          </div>
        )}
        {state === "error" && (
          <>
            <p className="tc-alert m-0" role="alert">
              {error}
            </p>
            <GlassButton className="justify-self-start" onClick={onRetry}>
              Retry
            </GlassButton>
          </>
        )}
        {state === "ready" && (
          <ul className="tc-card tc-card--quiet m-0 grid gap-2 font-mono text-xs capitalize">
            {names.map((name) => (
              <li key={name}>{name.replaceAll("_", " ")}</li>
            ))}
          </ul>
        )}
        <p className="text-sm text-tc-secondary">
          Scrubbing is good and it is not perfect. That is why you review each
          session before contributing it.
        </p>
      </div>
    </ResponsiveOverlay>
  );
}
