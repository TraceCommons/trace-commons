import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import { Button } from "../components/ui/button";
import {
  acknowledgeLegacyInviteMigration,
  getLegacyMigrationNotice,
} from "../lib/tauri/core-api";
import { parseLegacyMigrationStatus } from "../lib/tauri/legacy-migration";
import { coreKeys } from "../lib/tauri/query-keys";

/**
 * After a legacy invite identity moved to a NEAR AI account: the
 * contributor is told, wherever they are, until they acknowledge it. The
 * consent spec requires this in every shell; the words are the core's
 * (`consent_copy`), and this component only lays them out.
 */
export function LegacyMigrationNotice({ status }: { status: unknown }) {
  const queryClient = useQueryClient();
  let notice: Record<string, unknown> | null = null;
  let unreadable = false;
  try {
    notice = parseLegacyMigrationStatus(status).notice;
  } catch {
    unreadable = true;
  }
  const copy = useQuery({
    queryKey: ["contributor-copy", "legacy-migration-notice", notice],
    queryFn: () =>
      notice ? getLegacyMigrationNotice(notice) : Promise.resolve(null),
    enabled: notice !== null,
  });
  const acknowledge = useMutation({
    mutationFn: acknowledgeLegacyInviteMigration,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: coreKeys.status }),
  });
  if (unreadable || notice === null || !copy.data) return null;
  return (
    <Alert className="mx-6 mt-4 w-auto border-primary/40 bg-primary/10">
      <AlertTitle>{copy.data.title}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <span>{copy.data.body}</span>
        <span>{copy.data.folders}</span>
        <div>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={acknowledge.isPending}
            onClick={() => acknowledge.mutate()}
          >
            {copy.data.acknowledge}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}
