import { useCallback, useEffect, useRef, useState } from "react";
import {
  type NativeWalletView,
  nativeWalletFlow,
  openNativeWalletUrl,
} from "../api/onboarding-api";

export function useOnboardingWallet(onEnrolled: () => void) {
  const [flow, setFlow] = useState<NativeWalletView | null>(null);
  const [commons, setCommons] = useState("");
  const [account, setAccount] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const flowId = useRef("");
  const closed = useRef(false);

  useEffect(() => {
    closed.current = false;
    void nativeWalletFlow({ action: "open" })
      .then((next) => {
        flowId.current = next.flow_id;
        if (closed.current) {
          if (next.flow_id) {
            void nativeWalletFlow({ action: "cancel", flowId: next.flow_id });
          }
          return;
        }
        setFlow(next);
      })
      .catch(() => {
        if (!closed.current)
          setError("Wallet signup is unavailable in this build.");
      });
    return () => {
      closed.current = true;
      if (flowId.current) {
        void nativeWalletFlow({ action: "cancel", flowId: flowId.current });
      }
    };
  }, []);

  const run = useCallback(
    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Native wallet lifecycle mirrors the Rust-owned check/start/wait/cancel contract.
    async (action: "check" | "start" | "cancel") => {
      if (!flowId.current) return;
      setPending(true);
      setError(null);
      try {
        let next = await nativeWalletFlow({
          action,
          flowId: flowId.current,
          commons,
          account,
        });
        flowId.current = next.flow_id;
        if (closed.current) return;
        setFlow(next);
        if (action === "start" && next.browser_url) {
          try {
            await openNativeWalletUrl(next.browser_url);
          } catch (error) {
            const cancelled = await nativeWalletFlow({
              action: "cancel",
              flowId: next.flow_id,
            }).catch(() => null);
            if (cancelled && !closed.current) {
              flowId.current = cancelled.flow_id;
              setFlow(cancelled);
            }
            throw error;
          }
        }
        while (!closed.current && next.wait) {
          next = await nativeWalletFlow({
            action: "wait",
            flowId: next.flow_id,
          });
          flowId.current = next.flow_id;
          if (closed.current) break;
          setFlow(next);
        }
        if (!closed.current && next.state === "Complete") {
          setAccount("");
          onEnrolled();
        }
      } catch {
        setError(
          action === "cancel"
            ? "Wallet signup could not be cancelled."
            : "Wallet signup could not be completed. Check commons support and try again.",
        );
      } finally {
        if (!closed.current) setPending(false);
      }
    },
    [account, commons, onEnrolled],
  );

  return {
    flow,
    commons,
    setCommons,
    account,
    setAccount,
    pending,
    error,
    run,
  };
}
