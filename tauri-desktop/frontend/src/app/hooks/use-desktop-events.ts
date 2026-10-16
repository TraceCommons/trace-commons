import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { isTauriRuntime, listenTauri } from "../../lib/tauri/core-api";
import { consumeDeepLink, openExternalUrl } from "../../lib/tauri/platform-api";
import { isNavigablePath, routePaths } from "../routes";

type DesktopDeepLink = NonNullable<Awaited<ReturnType<typeof consumeDeepLink>>>;
type Navigate = ReturnType<typeof useNavigate>;

let initialPendingLinkRead: Promise<DesktopDeepLink | null> | null = null;
let initialPendingLinkHandled = false;

function readInitialPendingLink(): Promise<DesktopDeepLink | null> {
  if (initialPendingLinkRead === null) {
    initialPendingLinkRead = consumeDeepLink();
  }
  return initialPendingLinkRead;
}

async function handleDesktopLink(
  link: DesktopDeepLink,
  navigate: Navigate,
  setInvite: (invite: string) => void,
  onCredentialCallback: () => void,
): Promise<void> {
  switch (link.kind) {
    case "enroll":
      setInvite(link.invite);
      navigate(routePaths.profile);
      return;
    case "credential":
      onCredentialCallback();
      navigate(routePaths["private-ai"]);
      return;
    case "navigate":
      navigate(link.path);
      return;
    case "public_run":
      await openExternalUrl(link.url);
  }
}

async function registerTauriListener(
  event: string,
  handler: (payload: unknown) => void,
  isCancelled: () => boolean,
  cleanups: Array<() => void>,
): Promise<void> {
  const unlisten = await listenTauri(event, handler);
  if (isCancelled()) unlisten();
  else cleanups.push(unlisten);
}

async function consumeAndOpenDesktopLink(
  initial: boolean,
  isCancelled: () => boolean,
  navigate: Navigate,
  setInvite: (invite: string) => void,
  onCredentialCallback: () => void,
  setError: (error: string | null) => void,
): Promise<void> {
  try {
    if (isCancelled() || (initial && initialPendingLinkHandled)) return;
    const link = initial
      ? await readInitialPendingLink()
      : await consumeDeepLink();
    if (isCancelled() || !link) return;
    if (initial) initialPendingLinkHandled = true;
    setError(null);
    await handleDesktopLink(link, navigate, setInvite, onCredentialCallback);
  } catch {
    if (!isCancelled()) setError("A desktop link could not be opened.");
  }
}

export function useDesktopEvents(
  onCredentialCallback: () => void,
  missionsEnabled: boolean,
): {
  initialInvite: string | null;
  quitRequested: boolean;
  setQuitRequested: (open: boolean) => void;
  error: string | null;
} {
  const navigate = useNavigate();
  const [initialInvite, setInitialInvite] = useState<string | null>(null);
  const [quitRequested, setQuitRequested] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const credentialCallback = useRef(onCredentialCallback);
  const missionsEnabledRef = useRef(missionsEnabled);

  useEffect(() => {
    credentialCallback.current = onCredentialCallback;
  }, [onCredentialCallback]);

  useEffect(() => {
    missionsEnabledRef.current = missionsEnabled;
  }, [missionsEnabled]);

  useEffect(() => {
    let cancelled = false;
    const cleanups: Array<() => void> = [];
    const consume = (initial = false): void => {
      if (!isTauriRuntime()) return;
      void consumeAndOpenDesktopLink(
        initial,
        () => cancelled,
        navigate,
        setInitialInvite,
        credentialCallback.current,
        setError,
      );
    };
    const listen = async () => {
      const isCancelled = () => cancelled;
      await registerTauriListener(
        "deep-link-received",
        () => void consume(),
        isCancelled,
        cleanups,
      );
      await registerTauriListener(
        "navigate",
        (path) => {
          if (!cancelled && isNavigablePath(path, missionsEnabledRef.current))
            navigate(path);
        },
        isCancelled,
        cleanups,
      );
      await registerTauriListener(
        "quit-requested",
        () => {
          if (!cancelled) setQuitRequested(true);
        },
        isCancelled,
        cleanups,
      );
      if (!cancelled) setError(null);
      await consume(true);
    };

    void listen().catch(() => {
      for (const cleanup of cleanups.splice(0)) cleanup();
      if (!cancelled) setError("Desktop event listeners are unavailable.");
    });
    return () => {
      cancelled = true;
      for (const cleanup of cleanups) cleanup();
    };
  }, [navigate]);

  return { initialInvite, quitRequested, setQuitRequested, error };
}
