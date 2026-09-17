import { Button } from "@/components/ui/button";
import type { UseFormReturn } from "react-hook-form";
import type { PrivateAiProviderValues } from "../forms";
import type { usePrivateAi } from "../hooks/use-private-ai";

type PrivateAiController = ReturnType<typeof usePrivateAi>;

export function PrivateAiCredentialAction({
  privateAi,
  form,
}: {
  privateAi: PrivateAiController;
  form: UseFormReturn<PrivateAiProviderValues>;
}) {
  const action = privateAi.credential?.view?.action;
  if (action === "cancel") {
    return (
      <Button
        className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
        type="button"
        onClick={() => void privateAi.cancel()}
        disabled={privateAi.busy}
      >
        Cancel sign-in
      </Button>
    );
  }
  if (action === "forget") {
    return (
      <Button
        className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
        type="button"
        onClick={() => void privateAi.forget()}
        disabled={privateAi.busy}
      >
        Forget local credential
      </Button>
    );
  }
  if (action !== "obtain") return null;
  return (
    <Button
      className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
      type="submit"
      disabled={privateAi.busy || !form.formState.isValid}
    >
      Connect credential
    </Button>
  );
}
