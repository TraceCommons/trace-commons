import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { Button } from "../../../components/ui/button";
import { toast } from "../../../components/ui/toast";
import { accountAttentionWire } from "../../../lib/tauri/account-attention";
import { setUpAgain } from "../../../lib/tauri/contributor-copy-api";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useAccountAttentionCopy } from "../../../lib/tauri/use-contributor-copy";

/**
 * The account section's status while the daemon says this app's account
 * needs attention: a status in the section, not a banner or notification.
 * The words, the button and its confirmation are the contributor core's.
 * Only the confirmation's own button unenrolls; the first run then opens by
 * itself, because the daemon is no longer logged in.
 */
export function AccountAttentionStatus({ daemon }: { daemon: unknown }) {
  const copy = useAccountAttentionCopy(accountAttentionWire(daemon)).data;
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [failed, setFailed] = useState(false);
  // One run at a time, however often the button is pressed.
  const running = useRef(false);
  const [busy, setBusy] = useState(false);

  if (!copy) return null;

  const confirm = async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setFailed(false);
    try {
      const { leftBehind } = await setUpAgain();
      setConfirming(false);
      // Said only when the NEAR AI sign-in did not go with the enrollment.
      if (leftBehind) toast.add({ description: leftBehind });
      await queryClient.invalidateQueries({ queryKey: coreKeys.status });
    } catch {
      setFailed(true);
    } finally {
      running.current = false;
      setBusy(false);
    }
  };

  return (
    <div
      className="mb-5 rounded-xl border border-destructive/30 bg-destructive/5 p-4"
      role="status"
    >
      <strong className="block text-sm">{copy.title}</strong>
      {copy.body && (
        <p className="m-0 mt-1 text-[12px] leading-[1.55] text-muted-foreground">
          {copy.body}
        </p>
      )}
      {copy.private_ai && (
        <p className="m-0 mt-1 text-[12px] leading-[1.55] text-muted-foreground">
          {copy.private_ai}
        </p>
      )}
      <Button
        type="button"
        variant="outline"
        className="mt-3"
        disabled={busy}
        onClick={() => {
          setFailed(false);
          setConfirming(true);
        }}
      >
        {copy.action}
      </Button>
      {failed && !confirming && (
        <p className="m-0 mt-2 text-[12px] text-destructive">{copy.failed}</p>
      )}
      <ResponsiveOverlay
        open={confirming}
        onOpenChange={(open) => {
          if (!busy) setConfirming(open);
        }}
        title={copy.confirm_title}
        description={copy.confirm_body}
        footer={
          <>
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={() => setConfirming(false)}
            >
              {copy.confirm_cancel}
            </Button>
            <Button
              type="button"
              disabled={busy}
              onClick={() => void confirm()}
            >
              {copy.confirm_action}
            </Button>
          </>
        }
      >
        {failed && (
          <p className="text-sm text-destructive">{copy.failed}</p>
        )}
      </ResponsiveOverlay>
    </div>
  );
}
