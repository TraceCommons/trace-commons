import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { useSettings } from "../../settings/hooks/use-settings";
import {
  type PrivateAiProviderValues,
  privateAiProviderSchema,
} from "../forms";
import type { usePrivateAi } from "../hooks/use-private-ai";
import { PrivateAiCredentialAction } from "./private-ai-credential-action";

type PrivateAiController = ReturnType<typeof usePrivateAi>;
type SettingsController = ReturnType<typeof useSettings>;

export function PrivateAiConnectionPanel({
  privateAi,
  settings,
}: {
  privateAi: PrivateAiController;
  settings: SettingsController;
}) {
  const browserUrl = privateAi.browserUrl;
  const inferenceEnabled = settings.data?.private_inference === true;
  const canEnable =
    privateAi.busy || settings.data?.near_ai_inference_configured !== true;
  const form = useForm<PrivateAiProviderValues>({
    resolver: zodResolver(privateAiProviderSchema),
    defaultValues: { provider: "github" },
  });
  const providerError = form.formState.errors.provider?.message;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            CONNECTION
          </span>
          <h2>Private inference setup</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => {
            void settings.refresh();
            void privateAi.refreshCredential();
          }}
          disabled={settings.state === "loading" || privateAi.busy}
        >
          Refresh
        </Button>
      </div>
      <p>
        The production flow keeps privacy-filter credentials, inference
        credentials, and retained sessions separate. This surface reports their
        presence and daemon runtime state without exposing secrets.
      </p>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Enabling private inference starts a local listener for configured tools.
        It does not publish traces. Credential enrollment remains separate.
      </p>
      {privateAi.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {privateAi.error}
        </p>
      )}
      <form
        onSubmit={form.handleSubmit((values) => {
          if (privateAi.credential?.view?.action === "obtain")
            void privateAi.start(values.provider);
        })}
      >
        <div className="my-5 flex flex-wrap gap-2">
          <label>
            <span className="sr-only">Credential provider</span>
            <NativeSelect
              {...form.register("provider")}
              disabled={privateAi.busy}
              aria-invalid={Boolean(providerError)}
              aria-describedby={
                providerError ? "private-ai-provider-error" : undefined
              }
            >
              <option value="github">GitHub</option>
              <option value="google">Google</option>
              <option value="near">NEAR wallet</option>
            </NativeSelect>
            <FormFieldError
              id="private-ai-provider-error"
              message={providerError}
            />
          </label>
          <PrivateAiCredentialAction privateAi={privateAi} form={form} />
        </div>
      </form>
      {browserUrl && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Open sign-in:{" "}
          <Button
            className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
            type="button"
            onClick={() => void privateAi.openBrowser(browserUrl)}
            disabled={privateAi.busy}
          >
            continue in browser
          </Button>
          . URL is returned once by Rust.
        </p>
      )}
      {privateAi.credential?.keychain && (
        <div className="grid gap-1 border-t border-border pt-4 text-[11px] leading-[1.55] text-muted-foreground">
          <strong className="text-foreground">OS credential storage</strong>
          <span>
            {privateAi.credential.keychain.state} · migration{" "}
            {privateAi.credential.keychain.migration}
          </span>
          {privateAi.credential.keychain.key_prefix && (
            <span>
              Inference key prefix: {privateAi.credential.keychain.key_prefix}
            </span>
          )}
          {privateAi.credential.keychain.session_expires_at && (
            <span>
              Session expiry: {privateAi.credential.keychain.session_expires_at}
            </span>
          )}
        </div>
      )}
      <div className="flex items-center justify-between gap-6 border-t border-border py-[15px] first:mt-5">
        <div>
          <strong>Answer model calls on this computer</strong>
          <span>
            {inferenceEnabled ? "Enabled by explicit local choice" : "Disabled"}
          </span>
        </div>
        <Button
          variant={inferenceEnabled ? "secondary" : "default"}
          type="button"
          onClick={() => void privateAi.setEnabled(!inferenceEnabled)}
          disabled={canEnable}
        >
          {privateAi.busy
            ? "Applying…"
            : inferenceEnabled
              ? "Disable"
              : "Enable"}
        </Button>
      </div>
    </section>
  );
}
