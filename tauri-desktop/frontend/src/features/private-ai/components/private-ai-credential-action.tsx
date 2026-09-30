import type { UseFormReturn } from "react-hook-form";
import type { PrivateAiProviderValues } from "../forms";
import type { usePrivateAi } from "../hooks/use-private-ai";
import { GlassButton } from "@/design-system";

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
      <GlassButton
        type="button"
        onClick={() => void privateAi.cancel()}
        disabled={privateAi.busy}
      >
        Cancel sign-in
      </GlassButton>
    );
  }
  if (action === "forget") {
    return (
      <GlassButton
        className="tc-text-outside"
        type="button"
        onClick={() => void privateAi.forget()}
        disabled={privateAi.busy}
      >
        Forget local credential
      </GlassButton>
    );
  }
  if (action !== "obtain") return null;
  return (
    <GlassButton
      type="submit"
      disabled={privateAi.busy || !form.formState.isValid || !copyReady}
    >
      Connect credential
    </GlassButton>
  );
}
