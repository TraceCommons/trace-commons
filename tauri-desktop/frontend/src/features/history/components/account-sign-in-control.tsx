import { Button } from "@/components/ui/button";

export function AccountSignInControl({
  checking,
  pending,
  openingFallback,
  canSignIn,
  signInUrlAvailable,
  error,
  onSignIn,
  onOpenFallback,
}: {
  checking: boolean;
  pending: boolean;
  openingFallback: boolean;
  canSignIn: boolean;
  signInUrlAvailable: boolean;
  error: string | null;
  onSignIn?: () => void;
  onOpenFallback?: () => void;
}) {
  if (checking) {
    return (
      <span className="text-xs text-muted-foreground" role="status">
        Checking account session…
      </span>
    );
  }

  return (
    <div className="grid justify-items-end gap-1.5">
      <Button
        type="button"
        variant="outline"
        onClick={onSignIn}
        disabled={!canSignIn || pending || !onSignIn}
      >
        {pending ? "Waiting for sign-in…" : "Sign in to withdraw"}
      </Button>
      {pending && (
        <span className="max-w-64 text-right text-xs text-muted-foreground" role="status">
          Complete sign-in in your browser. This may take up to five minutes.
        </span>
      )}
      {pending && signInUrlAvailable && (
        <Button
          type="button"
          variant="link"
          className="h-auto p-0 text-xs"
          onClick={onOpenFallback}
          disabled={openingFallback || !onOpenFallback}
        >
          {openingFallback ? "Opening sign-in page…" : "Open sign-in page again"}
        </Button>
      )}
      {error && (
        <span className="max-w-64 text-right text-xs text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
