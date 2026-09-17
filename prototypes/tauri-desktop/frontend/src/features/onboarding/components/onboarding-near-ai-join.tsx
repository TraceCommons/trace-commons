import { useEffect } from "react";
import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useOnboardingNearAi } from "../hooks/use-onboarding-near-ai";

export function OnboardingNearAiJoin({
  onEnrolled,
  onBusyChanged,
  blocked = false,
}: {
  onEnrolled: () => void;
  onBusyChanged?: (busy: boolean) => void;
  blocked?: boolean;
}) {
  const nearAi = useOnboardingNearAi(onEnrolled);
  useEffect(() => {
    onBusyChanged?.(nearAi.busy);
  }, [nearAi.busy, onBusyChanged]);
  return (
    <section className="grid gap-4 border-t border-border pt-5">
      <div>
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          NEAR AI
        </span>
        <h3>Join with existing NEAR AI sign-in</h3>
        <p className="m-0 text-[12px] leading-[1.55] text-muted-foreground">
          This path needs no wallet. Rust verifies your existing NEAR AI session
          with the commons before enrollment.
        </p>
      </div>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="near-ai-commons">Commons URL</FieldLabel>
          <Input
            id="near-ai-commons"
            value={nearAi.commons}
            onChange={(event) => nearAi.setCommons(event.target.value)}
            placeholder="https://commons.example"
            disabled={nearAi.busy || blocked}
          />
          <FieldDescription>Used only when you press Join.</FieldDescription>
        </Field>
      </FieldGroup>
      {nearAi.credential.isPending && (
        <p className="m-0 text-[12px] text-muted-foreground">
          Checking NEAR AI sign-in status…
        </p>
      )}
      {nearAi.signedIn ? (
        <Button
          type="button"
          onClick={() => nearAi.join.mutate()}
          disabled={nearAi.busy || blocked || !nearAi.commons.trim()}
        >
          {nearAi.join.isPending ? "Joining…" : "Join with NEAR AI"}
        </Button>
      ) : (
        <div className="grid gap-3">
          <p className="m-0 text-[12px] text-muted-foreground">
            Sign in to NEAR AI first. Session status stays local.
          </p>
          <div className="flex flex-wrap items-end gap-2.5">
            <Field>
              <FieldLabel htmlFor="near-ai-provider">Provider</FieldLabel>
              <NativeSelect
                id="near-ai-provider"
                value={nearAi.provider}
                onChange={(event) => nearAi.setProvider(event.target.value)}
                disabled={nearAi.busy || blocked}
              >
                <option value="github">GitHub</option>
                <option value="google">Google</option>
                <option value="near">NEAR wallet</option>
              </NativeSelect>
            </Field>
            <Button
              type="button"
              variant="outline"
              onClick={() => nearAi.start.mutate()}
              disabled={nearAi.busy || blocked}
            >
              {nearAi.start.isPending ? "Starting…" : "Start sign-in"}
            </Button>
          </div>
          {nearAi.browserUrl && (
            <p className="m-0 text-[12px] text-muted-foreground">
              Open sign-in:{" "}
              <Button
                type="button"
                variant="link"
                className="h-auto p-0 text-[12px]"
                onClick={() => nearAi.open.mutate(nearAi.browserUrl ?? "")}
                disabled={nearAi.busy || blocked}
              >
                continue in browser
              </Button>
              .
            </p>
          )}
        </div>
      )}
      {nearAi.error && (
        <p className="m-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {nearAi.error}
        </p>
      )}
    </section>
  );
}
