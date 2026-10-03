import { type ReactNode, useEffect, useRef, useState } from "react";
import { Tag } from "../../../design-system";
import {
  createPasskey,
  signInWithPasskey,
  verifyPasskey,
} from "../api/ftux-api";
import {
  type PasskeyEvent,
  type PasskeyStep,
  passkeyNameError,
  passkeyTransition,
} from "../ftux-model";
import type { PasskeyStore } from "../types";
import { StatusNote } from "./ftux-frame";
import {
  BackIcon,
  CloseIcon,
  FingerprintIcon,
  LockIcon,
  PasskeyIcon,
  Spinner,
  WarningIcon,
} from "./icons";

export type PasskeyResult = { name: string; store: PasskeyStore };

const STORE_LABELS: Record<PasskeyStore, string> = {
  "1password": "1Password",
  passwords: "Passwords",
};

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])';

// A modal layer: everything beside it is made inert while it is open, Tab
// cycles inside it, and focus goes back where it came from when it closes.
// Remounted per step (see `key` below), so focus moves to each new popup.
function Overlay({
  label,
  onEscape,
  children,
}: {
  label: string;
  onEscape: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const layer = ref.current;
    if (!layer) return;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const madeInert = Array.from(layer.parentElement?.children ?? []).filter(
      (sibling): sibling is HTMLElement =>
        sibling !== layer &&
        sibling instanceof HTMLElement &&
        !sibling.hasAttribute("inert"),
    );
    for (const sibling of madeInert) sibling.setAttribute("inert", "");
    layer
      .querySelector<HTMLElement>("input, button:not([data-skip-focus])")
      ?.focus();
    return () => {
      for (const sibling of madeInert) sibling.removeAttribute("inert");
      if (opener?.isConnected) opener.focus();
      else
        layer.parentElement?.querySelector<HTMLElement>(".ftux-pane")?.focus();
    };
  }, []);
  return (
    <div
      ref={ref}
      className="tc-scrim ftux-scrim"
      role="dialog"
      aria-modal="true"
      aria-label={label}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onEscape();
          return;
        }
        if (event.key !== "Tab" || !ref.current) return;
        const focusable = Array.from(
          ref.current.querySelectorAll<HTMLElement>(FOCUSABLE),
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }}
    >
      {children}
    </div>
  );
}

function CornerButton({
  side,
  label,
  onClick,
}: {
  side: "left" | "right";
  label: "Back" | "Close";
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="tc-btn tc-btn--round tc-btn--small ftux-corner"
      data-side={side}
      data-skip-focus
      aria-label={label}
      onClick={onClick}
    >
      {label === "Back" ? <BackIcon /> : <CloseIcon />}
    </button>
  );
}

// The imitation macOS sheets say so on their face: a drawn copy of a system
// sheet must never pass for the real one.
function SimulatedMark() {
  return (
    <span className="ftux-simulated">
      <Tag tone="ask">Simulated · the real sheet is drawn by macOS</Tag>
    </span>
  );
}

function ErrorLine({ error }: { error: string | null }) {
  return error ? (
    <StatusNote tone="outside" role="alert">
      {error}
    </StatusNote>
  ) : null;
}

type StepProps = {
  send: (event: PasskeyEvent, result?: PasskeyResult) => void;
  run: (work: () => Promise<void>) => void;
  busy: boolean;
  error: string | null;
};

// P-1
function ChoosePopup({ send }: StepProps) {
  return (
    <div className="tc-modal tc-modal--narrow ftux-popup">
      <CornerButton
        side="right"
        label="Close"
        onClick={() => send({ type: "cancel" })}
      />
      <span className="ftux-popup-icon">
        <PasskeyIcon />
      </span>
      <div className="ftux-popup-heading">
        <h2 className="tc-heading m-0">Continue with passkey</h2>
      </div>
      <button
        type="button"
        className="tc-btn tc-btn--primary ftux-block-btn"
        onClick={() => send({ type: "use-existing" })}
      >
        Use existing passkey
      </button>
      <button
        type="button"
        className="tc-btn tc-btn--glass ftux-block-btn"
        onClick={() => send({ type: "create-new" })}
      >
        Create new passkey
      </button>
      <p className="m-0 tc-caption tc-text-tertiary ftux-center">
        A passkey is your sign-in for Trace Commons and near.ai. Nothing about
        your sessions is sent by signing in.
      </p>
    </div>
  );
}

// P-2
function NamePopup({
  send,
  name,
  onName,
}: StepProps & { name: string; onName: (name: string) => void }) {
  const [touched, setTouched] = useState(false);
  const nameError = passkeyNameError(name);
  const showError = touched && nameError !== null;
  return (
    <form
      className="tc-modal tc-modal--narrow ftux-popup"
      onSubmit={(event) => {
        event.preventDefault();
        setTouched(true);
        if (!nameError) send({ type: "named" });
      }}
    >
      <CornerButton
        side="left"
        label="Back"
        onClick={() => send({ type: "back" })}
      />
      <CornerButton
        side="right"
        label="Close"
        onClick={() => send({ type: "cancel" })}
      />
      <span className="ftux-popup-icon">
        <PasskeyIcon />
      </span>
      <div className="ftux-popup-heading">
        <h2 className="tc-heading m-0">Create new passkey</h2>
      </div>
      <div className="ftux-name-field">
        <input
          aria-label="Passkey name"
          aria-invalid={showError}
          value={name}
          maxLength={80}
          onChange={(event) => {
            onName(event.target.value);
            setTouched(true);
          }}
        />
        {name ? (
          <button
            type="button"
            className="ftux-clear"
            aria-label="Clear name"
            data-skip-focus
            onClick={() => onName("")}
          >
            <CloseIcon size={9} />
          </button>
        ) : null}
      </div>
      <ErrorLine error={showError ? nameError : null} />
      <button
        type="submit"
        className="tc-btn tc-btn--primary ftux-block-btn"
        disabled={showError}
      >
        Create new passkey
      </button>
      <StatusNote tone="ask" icon={<WarningIcon />}>
        Store your passkey securely. Losing it means losing access to your
        account and any credit in it.
      </StatusNote>
    </form>
  );
}

function StoreOption({
  store,
  checked,
  onChange,
}: {
  store: PasskeyStore;
  checked: boolean;
  onChange: () => void;
}) {
  return (
    <label className="ftux-sheet-option">
      <input
        type="radio"
        name="ftux-passkey-store"
        checked={checked}
        onChange={onChange}
      />
      <span className="ftux-app-icon" data-app={store} aria-hidden="true">
        {store === "1password" ? "1" : null}
      </span>
      Save in {STORE_LABELS[store]}
    </label>
  );
}

// P-3 and P-4. Simulated macOS sheet: the OS draws the real one when the
// WebAuthn request is made.
function SaveSheet({
  send,
  run,
  busy,
  error,
  touching,
  name,
  store,
  onStore,
  onCreated,
}: StepProps & {
  touching: boolean;
  name: string;
  store: PasskeyStore;
  onStore: (store: PasskeyStore) => void;
  onCreated: (result: PasskeyResult) => void;
}) {
  return (
    <div className="ftux-sheet">
      <SimulatedMark />
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <h2 className="ftux-sheet-title">Save a passkey?</h2>
        <p className="ftux-sheet-text">
          “tracecommons.ai” supports passkeys, a stronger alternative to
          passwords that cannot be leaked or stolen.
        </p>
      </div>
      <fieldset className="ftux-sheet-group" disabled={touching}>
        <legend className="sr-only">Where to save the passkey</legend>
        {(["1password", "passwords"] as const).map((option) => (
          <StoreOption
            key={option}
            store={option}
            checked={store === option}
            onChange={() => onStore(option)}
          />
        ))}
      </fieldset>
      {touching ? (
        <button
          type="button"
          className="ftux-touch-id"
          data-busy={busy}
          disabled={busy}
          onClick={() =>
            run(async () => {
              onCreated(await createPasskey(name, store));
              send({ type: "touched" });
            })
          }
        >
          <FingerprintIcon />
          Touch ID to Save Passkey
        </button>
      ) : null}
      <ErrorLine error={error} />
      <div className="ftux-sheet-buttons">
        <button
          type="button"
          className="ftux-sheet-btn"
          data-skip-focus
          disabled={busy}
          onClick={() => send({ type: "cancel" })}
        >
          Cancel
        </button>
        {touching ? null : (
          <button
            type="button"
            className="ftux-sheet-btn"
            data-default="true"
            onClick={() => send({ type: "store-chosen" })}
          >
            Continue
          </button>
        )}
      </div>
    </div>
  );
}

// P-5. Binds the passkey to the near.ai account.
function VerifyPopup({ send, run, busy, error }: StepProps) {
  return (
    <div className="tc-modal tc-modal--narrow ftux-popup">
      <span className="ftux-popup-icon">
        <LockIcon />
      </span>
      <div className="ftux-popup-heading">
        <h2 className="tc-heading m-0">Verify your passkey</h2>
        <p className="m-0 tc-text-secondary">
          Sign a message to prove the passkey is yours and unlock contributing
          and credit.
        </p>
      </div>
      <ErrorLine error={error} />
      <button
        type="button"
        className="tc-btn tc-btn--glass ftux-block-btn ftux-btn-outline"
        disabled={busy}
        onClick={() =>
          run(async () => {
            await verifyPasskey();
            send({ type: "verified" });
          })
        }
      >
        {busy ? <Spinner /> : null}
        Verify passkey
      </button>
      <button
        type="button"
        className="tc-btn tc-btn--glass ftux-block-btn"
        disabled={busy}
        onClick={() => send({ type: "cancel" })}
      >
        Cancel
      </button>
      <p className="m-0 tc-caption tc-text-tertiary ftux-center">
        Cancelling signs you out.
      </p>
    </div>
  );
}

// P-6. Simulated macOS "Sign In" sheet. It names a passkey only when one is
// known to be stored here; otherwise macOS would list what it finds.
function SignInSheet({
  send,
  run,
  busy,
  error,
  stored,
}: StepProps & { stored: PasskeyResult | null }) {
  return (
    <div className="ftux-sheet">
      <SimulatedMark />
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <h2 className="ftux-sheet-title">Sign In</h2>
        <p className="ftux-sheet-text">
          {stored
            ? `Sign in to “tracecommons.ai” with your passkey for “${stored.name}” saved in “${STORE_LABELS[stored.store]}”?`
            : "Sign in to “tracecommons.ai” with a saved passkey?"}
        </p>
      </div>
      {stored ? (
        <div
          className="ftux-sheet-group ftux-card-row"
          style={{ flexDirection: "row", padding: "10px 14px", fontSize: 13 }}
        >
          <span className="tc-text-tertiary">Passkey from</span>
          <span
            style={{ display: "inline-flex", alignItems: "center", gap: 8 }}
          >
            <span
              className="ftux-app-icon"
              data-app={stored.store}
              aria-hidden="true"
            >
              {stored.store === "1password" ? "1" : null}
            </span>
            {STORE_LABELS[stored.store]}
          </span>
        </div>
      ) : null}
      <button
        type="button"
        className="ftux-touch-id"
        data-busy={busy}
        disabled={busy}
        onClick={() =>
          run(async () => {
            send({ type: "touched" }, await signInWithPasskey());
          })
        }
      >
        <FingerprintIcon />
        Touch ID to Use Passkey
      </button>
      <ErrorLine error={error} />
      <div className="ftux-sheet-buttons">
        <button
          type="button"
          className="ftux-sheet-btn"
          data-skip-focus
          disabled={busy}
          onClick={() => send({ type: "cancel" })}
        >
          Cancel
        </button>
        <button
          type="button"
          className="ftux-sheet-btn"
          data-skip-focus
          disabled={busy}
          onClick={() => send({ type: "cancel" })}
        >
          More Options
        </button>
      </div>
    </div>
  );
}

const STEP_LABELS: Record<PasskeyStep, string> = {
  choose: "Continue with passkey",
  name: "Create new passkey",
  "save-where": "Save a passkey?",
  "touch-id": "Save a passkey?",
  verify: "Verify your passkey",
  "sign-in": "Sign in",
};

// The Join screen's "Create passkey" opens this stack of popups. The whole
// stack runs on mock data until WebAuthn for tracecommons.ai exists.
export function PasskeyFlow({
  initialStep = "choose",
  storedPasskey = null,
  onDone,
  onClose,
}: {
  initialStep?: PasskeyStep;
  // A passkey known to be stored on this Mac, named on the Sign In sheet.
  storedPasskey?: PasskeyResult | null;
  onDone: (result: PasskeyResult) => void;
  onClose: (reason: "closed" | "signed-out") => void;
}) {
  const [step, setStep] = useState<PasskeyStep>(initialStep);
  const [name, setName] = useState("My trace passkey");
  const [store, setStore] = useState<PasskeyStore>("1password");
  const [created, setCreated] = useState<PasskeyResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const send = (event: PasskeyEvent, result?: PasskeyResult) => {
    const next = passkeyTransition(step, event);
    setError(null);
    if (next.kind === "step") {
      setStep(next.step);
      return;
    }
    if (next.kind === "done") {
      const final = result ?? created;
      if (final) onDone(final);
      return;
    }
    onClose(next.kind);
  };

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong.");
    } finally {
      setBusy(false);
    }
  };

  const props: StepProps = {
    send,
    run: (work) => void run(work),
    busy,
    error,
  };
  const body = {
    choose: <ChoosePopup {...props} />,
    name: <NamePopup {...props} name={name} onName={setName} />,
    "save-where": (
      <SaveSheet
        {...props}
        touching={false}
        name={name}
        store={store}
        onStore={setStore}
        onCreated={setCreated}
      />
    ),
    "touch-id": (
      <SaveSheet
        {...props}
        touching
        name={name}
        store={store}
        onStore={setStore}
        onCreated={setCreated}
      />
    ),
    verify: <VerifyPopup {...props} />,
    "sign-in": <SignInSheet {...props} stored={storedPasskey} />,
  }[step];

  return (
    <Overlay
      key={step}
      label={STEP_LABELS[step]}
      onEscape={() => {
        if (!busy) send({ type: "cancel" });
      }}
    >
      {body}
    </Overlay>
  );
}

// P-7: a returning user sees the stored passkey first.
export function WelcomeBack({
  passkeyName,
  onSignIn,
  onOtherOptions,
}: {
  passkeyName: string;
  onSignIn: () => void;
  onOtherOptions: () => void;
}) {
  return (
    <Overlay label="Welcome back" onEscape={onOtherOptions}>
      <div className="tc-modal tc-modal--narrow ftux-popup">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 className="tc-display m-0">Welcome back</h2>
          <p className="m-0 tc-text-secondary">Sign in with your passkey.</p>
        </div>
        <div
          className="tc-card ftux-welcome-card"
          style={{ alignItems: "center", gap: 14, padding: "20px 16px" }}
        >
          <span className="ftux-popup-icon" data-tone="green">
            <FingerprintIcon size={24} strokeWidth={1.8} />
          </span>
          <span style={{ fontSize: 16, fontWeight: 600 }}>{passkeyName}</span>
          <button
            type="button"
            className="tc-btn tc-btn--primary ftux-block-btn"
            onClick={onSignIn}
          >
            Sign in with passkey
          </button>
        </div>
        <button
          type="button"
          className="tc-link ftux-self-center"
          style={{ alignSelf: "center", fontSize: 13 }}
          onClick={onOtherOptions}
        >
          Other sign-in options
        </button>
      </div>
    </Overlay>
  );
}
