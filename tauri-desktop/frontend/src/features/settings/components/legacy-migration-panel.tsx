import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  getLegacyMigrationOffer,
  migrateLegacyInvite,
} from "../../../lib/tauri/core-api";
import {
  type LegacyMigrationResult,
  parseLegacyMigrationStatus,
} from "../../../lib/tauri/legacy-migration";
import { coreKeys } from "../../../lib/tauri/query-keys";

/**
 * The opt-in to move a legacy invite identity to the contributor's NEAR AI
 * account. Shown only while the core offers it; nothing moves until the
 * contributor presses the button, and declining changes nothing. Every word
 * is the core's (`consent_copy`); a refusal shows the core's sentence for
 * its label, and the invite is asked for only when the core says it needs
 * it.
 */
export function LegacyMigrationPanel({ status }: { status: unknown }) {
  const queryClient = useQueryClient();
  const [invite, setInvite] = useState("");
  const [answer, setAnswer] = useState<LegacyMigrationResult | null>(null);
  let offered = false;
  try {
    offered = parseLegacyMigrationStatus(status).offered;
  } catch {
    offered = false;
  }
  const copy = useQuery({
    queryKey: ["contributor-copy", "legacy-migration-offer"],
    queryFn: getLegacyMigrationOffer,
    enabled: offered,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const migrate = useMutation({
    mutationFn: (pasted: string | null) => migrateLegacyInvite(pasted),
    onSuccess: (result) => {
      setAnswer(result);
      if (result.kind === "migrated") {
        void queryClient.invalidateQueries({ queryKey: coreKeys.status });
      }
    },
  });
  if (!offered || !copy.data) return null;
  const needsInvite = answer?.kind === "invite_needed";
  return (
    <section className="mb-4 tc-card">
      <span className="mb-1.5 block tc-eyebrow">
        ACCOUNT
      </span>
      <h2>{copy.data.title}</h2>
      <p className="mt-2 text-[13px] leading-[1.55] text-muted-foreground">
        {copy.data.body}
      </p>
      {needsInvite && (
        <div className="mt-4 grid gap-2 text-[13px]">
          <label htmlFor="legacy-migration-invite">
            {copy.data.invite_prompt}
          </label>
          <Input
            id="legacy-migration-invite"
            value={invite}
            onChange={(event) => setInvite(event.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
        </div>
      )}
      <div className="mt-4 flex flex-wrap items-center gap-3">
        <Button
          type="button"
          size="sm"
          disabled={migrate.isPending || (needsInvite && invite.trim() === "")}
          onClick={() => migrate.mutate(needsInvite ? invite.trim() : null)}
        >
          {migrate.isPending ? copy.data.working : copy.data.action}
        </Button>
      </div>
      {answer && answer.kind !== "migrated" && (
        <p className="mt-3 text-[13px] text-muted-foreground" role="status">
          {answer.line}
        </p>
      )}
      {migrate.isError && (
        <p className="mt-3 text-[13px] text-destructive" role="alert">
          {copy.data.start_failed}
        </p>
      )}
    </section>
  );
}
