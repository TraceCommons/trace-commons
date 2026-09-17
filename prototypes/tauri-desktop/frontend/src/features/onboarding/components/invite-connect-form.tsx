import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { FormFieldError } from "../../../components/form-field-error";
import { type ResolvedInvite, resolveInvite } from "../api/invite-link";
import { type InviteFormValues, inviteFormSchema } from "../forms";

export function InviteConnectForm({
  busy,
  initialInvite,
  onConnect,
}: {
  busy: boolean;
  initialInvite?: string | null;
  onConnect: (invite: string) => Promise<void>;
}) {
  const form = useForm<InviteFormValues>({
    resolver: zodResolver(inviteFormSchema),
    defaultValues: { invite: initialInvite ?? "" },
    mode: "onChange",
  });
  const [resolved, setResolved] = useState<ResolvedInvite | null>(null);
  const [invalid, setInvalid] = useState(false);
  const [joining, setJoining] = useState(false);
  const inviteError = form.formState.errors.invite?.message;

  const resolve = () => {
    const link = resolveInvite(form.getValues("invite"));
    setResolved(link);
    setInvalid(link === null);
  };

  useEffect(() => {
    if (!initialInvite) return;
    form.setValue("invite", initialInvite, { shouldValidate: true });
    const link = resolveInvite(initialInvite);
    setResolved(link);
    setInvalid(link === null);
  }, [form, initialInvite]);

  const join = async () => {
    if (!resolved) return;
    setJoining(true);
    try {
      await onConnect(resolved.value);
    } finally {
      setJoining(false);
    }
  };

  return (
    <form className="grid gap-3" onSubmit={form.handleSubmit(resolve)}>
      <div className="grid gap-1.5">
        <label htmlFor="invite-link">Invite link</label>
        <Textarea
          id="invite-link"
          {...form.register("invite", {
            onChange: () => {
              setResolved(null);
              setInvalid(false);
            },
          })}
          rows={3}
          placeholder="Paste invite link"
          disabled={busy || joining}
          aria-invalid={Boolean(inviteError)}
          aria-describedby={inviteError ? "invite-error" : undefined}
        />
        <FormFieldError id="invite-error" message={inviteError} />
      </div>
      <div className="flex flex-wrap gap-2.5">
        <Button
          type="submit"
          variant="outline"
          disabled={busy || joining || !form.formState.isValid}
        >
          Look up
        </Button>
        {resolved && (
          <Button
            type="button"
            onClick={() => void join()}
            disabled={busy || joining}
          >
            {joining ? "Connecting…" : `Join ${resolved.issuerHost}`}
          </Button>
        )}
      </div>
      {invalid && (
        <p className="m-0 text-[12px] text-destructive">
          This invite link is no longer valid. Ask whoever sent it for a new
          one.
        </p>
      )}
      {resolved && (
        <p className="m-0 text-[12px] text-muted-foreground">
          This invite is for <strong>{resolved.issuerHost}</strong>.
        </p>
      )}
    </form>
  );
}
