import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { Alert, AlertDescription } from "../../../components/ui/alert";
import { Button } from "../../../components/ui/button";
import { Skeleton } from "../../../components/ui/skeleton";

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
        <Button type="button" onClick={onClose} className="w-full sm:w-auto">
          Close
        </Button>
      }
    >
      <div className="grid gap-4 pb-4">
        <p className="text-sm text-muted-foreground">
          Before anything leaves this machine, local scrubbing looks for these
          named secret categories. Names are shown; detector patterns stay
          private.
        </p>
        {state === "loading" && (
          <div className="grid gap-2" aria-label="Reading detector list">
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-4 w-1/2" />
            <Skeleton className="h-4 w-3/4" />
          </div>
        )}
        {state === "error" && (
          <>
            <Alert variant="destructive">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
            <Button type="button" variant="outline" onClick={onRetry}>
              Retry
            </Button>
          </>
        )}
        {state === "ready" && (
          <ul className="m-0 grid gap-2 rounded-lg border bg-muted/40 p-4 font-mono text-xs capitalize">
            {names.map((name) => (
              <li key={name}>{name.replaceAll("_", " ")}</li>
            ))}
          </ul>
        )}
        <p className="text-sm text-muted-foreground">
          Scrubbing is good and it is not perfect. That is why you review each
          session before contributing it.
        </p>
      </div>
    </ResponsiveOverlay>
  );
}
