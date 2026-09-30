import { Button } from "@/components/ui/button";
import type { UseFormReturn } from "react-hook-form";
import type { PrivateAiProviderValues } from "../forms";
import type { usePrivateAi } from "../hooks/use-private-ai";

type PrivateAiController = ReturnType<typeof usePrivateAi>;

export function PrivateAiCredentialAction({
  privateAi,
  form,
  copyReady,
}: {
  privateAi: PrivateAiController;
  form: UseFormReturn<PrivateAiProviderValues>;
  copyReady: boolean;
}) {
  const action = privateAi.credential?.view?.action;
  if (action === "cancel") {
    return (
      <Button
        className="tc-btn tc-btn--glass"
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
        className="tc-btn tc-btn--glass tc-text-outside"
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
      className="tc-btn tc-btn--glass"
      type="submit"
      disabled={privateAi.busy || !form.formState.isValid || !copyReady}
    >
      Connect credential
    </Button>
  );
}
