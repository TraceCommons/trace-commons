import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { RoutingEvidence } from "../api/routing-api";
import { type RoutingFormValues, routingFormSchema } from "../forms";

export function RoutingControls({
  enabled: initialEnabled,
  port: initialPort,
  tokenDir: initialTokenDir,
  busy,
  evidence,
  onCheck,
  onConfigure,
}: {
  enabled: boolean;
  port: number;
  tokenDir: string;
  busy: boolean;
  evidence: RoutingEvidence | null;
  onCheck: (port: number, tokenDir: string) => Promise<unknown>;
  onConfigure: (
    enabled: boolean,
    port: number,
    tokenDir: string,
  ) => Promise<unknown>;
}) {
  const form = useForm<RoutingFormValues>({
    resolver: zodResolver(routingFormSchema),
    defaultValues: {
      enabled: initialEnabled,
      port: String(initialPort),
      tokenDir: initialTokenDir,
    },
    mode: "onChange",
  });
  const enabled = form.watch("enabled");
  useEffect(() => {
    if (!form.formState.isDirty) {
      form.reset({
        enabled: initialEnabled,
        port: String(initialPort),
        tokenDir: initialTokenDir,
      });
    }
  }, [form, initialEnabled, initialPort, initialTokenDir]);
  const portError = form.formState.errors.port?.message;
  const tokenDirError = form.formState.errors.tokenDir?.message;
  const save = async (values: RoutingFormValues) => {
    try {
      await onConfigure(values.enabled, Number(values.port), values.tokenDir);
      form.reset(values);
    } catch {
      form.reset({
        enabled: initialEnabled,
        port: String(initialPort),
        tokenDir: initialTokenDir,
      });
    }
  };
  const check = form.handleSubmit((values) =>
    onCheck(Number(values.port), values.tokenDir),
  );
  return (
    <form onSubmit={form.handleSubmit(save)}>
      <label className="mt-5 flex items-start gap-2.5 text-[12px] font-normal text-foreground">
        <Checkbox
          checked={enabled}
          onCheckedChange={(checked) =>
            form.setValue("enabled", checked === true, { shouldDirty: true })
          }
          disabled={busy}
        />
        <span>
          <strong>Use declared local proxy</strong>
          <small>
            Turning this off clears the declaration and returns to no routing.
          </small>
        </span>
      </label>
      <div className="mt-[18px] grid grid-cols-[150px_minmax(0,1fr)] gap-3.5 max-[860px]:grid-cols-1">
        <label>
          Port
          <Input
            {...form.register("port")}
            type="number"
            min="1"
            max="65535"
            disabled={busy || !enabled}
            aria-invalid={Boolean(portError)}
            aria-describedby={portError ? "routing-port-error" : undefined}
          />
          <FormFieldError id="routing-port-error" message={portError} />
        </label>
        <label>
          Token directory
          <Input
            {...form.register("tokenDir")}
            placeholder="Optional absolute directory"
            disabled={busy || !enabled}
            aria-invalid={Boolean(tokenDirError)}
            aria-describedby={
              tokenDirError ? "routing-token-dir-error" : undefined
            }
          />
          <small>
            Directory only. Credential contents never enter settings UI.
          </small>
          <FormFieldError
            id="routing-token-dir-error"
            message={tokenDirError}
          />
        </label>
      </div>
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="submit"
          disabled={busy || !form.formState.isValid}
        >
          Save routing
        </Button>
        {enabled && (
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={() => void check()}
            disabled={busy || !form.formState.isValid}
          >
            Check now
          </Button>
        )}
      </div>
      {evidence && (
        <div className="mt-[22px] grid gap-px border-t border-border">
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.08em] text-primary">
            PROXY EVIDENCE · {evidence.outcome}
          </span>
          {evidence.tools.map((tool) => (
            <div key={tool.id}>
              <span>{tool.id}</span>
              <small>
                {tool.wired
                  ? "Connected to local proxy"
                  : tool.installed
                    ? "Installed, not connected"
                    : "Not installed"}
              </small>
            </div>
          ))}
        </div>
      )}
    </form>
  );
}
