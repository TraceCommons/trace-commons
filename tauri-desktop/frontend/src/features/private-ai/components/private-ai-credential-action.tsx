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
  if (action === "migrate") {
    // Both strings come from the core copy module through the status view;
    // without them nothing is drawn, because the sentence warns about the
    // macOS password prompt the button can cause.
    const label = privateAi.credential?.view?.action_label;
    const explains = privateAi.credential?.view?.action_explains;
    if (!label || !explains) return null;
    return (
      <div className="flex flex-col gap-2">
        <p className="m-0 text-[11px] text-tc-secondary">{explains}</p>
        <GlassButton
          type="button"
          onClick={() => void privateAi.migrate()}
          disabled={privateAi.busy}
        >
          {label}
        </GlassButton>
      </div>
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
