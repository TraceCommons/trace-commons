import { MOCK_INVITE_PLACEHOLDER } from "../api/ftux-mock-data";
import type { JoinState } from "../types";
import { CheckIcon, ExternalIcon, ScreenTitle, Spinner } from "./glass";

export type AsyncState = { status: "idle" | "busy" | "error"; error?: string };

export function JoinScreen({
  join,
  inviteDraft,
  lookup,
  nearAi,
  notice,
  onInviteDraft,
  onLookup,
  onCreatePasskey,
  onSignInNearAi,
  onNext,
  showPasskey = false,
}: {
  join: JoinState;
  inviteDraft: string;
  lookup: AsyncState;
  nearAi: AsyncState;
  notice: string | null;
  onInviteDraft: (value: string) => void;
  onLookup: () => void;
  onCreatePasskey: () => void;
  onSignInNearAi: () => void;
  onNext: () => void;
  // Hidden in the first release: under the consent spec (rev 8) near.ai is
  // the account, and a passkey waits until the account model defines it.
  showPasskey?: boolean;
}) {
  // Contributing needs an account; without one, setup is watching only.
  const hasAccount = join.nearAi || join.passkey !== null;
  return (
    <>
      <ScreenTitle light="Get started on " bold="your terms" />
      <p className="ftux-lede">
        Start with an invite link, sign-up or sign-in with an existing account,
        or just click "Skip".{" "}
        <b>
          You'll be able to setup or connect your account later to receive
          credits and manage access to the near.ai ecosystem.
        </b>
      </p>
      <div className="ftux-scroll">
        <form
          className="ftux-card"
          data-done={join.invite !== null}
          onSubmit={(event) => {
            event.preventDefault();
            onLookup();
          }}
        >
          <label className="ftux-section-label" htmlFor="ftux-invite">
            Invite link
          </label>
          <div style={{ display: "flex", gap: 8 }}>
            <input
              id="ftux-invite"
              className="ftux-input"
              value={inviteDraft}
              placeholder={`${MOCK_INVITE_PLACEHOLDER}…`}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => onInviteDraft(event.target.value)}
            />
            <button
              type="submit"
              className="ftux-btn"
              disabled={lookup.status === "busy" || !inviteDraft.trim()}
            >
              {lookup.status === "busy" ? <Spinner /> : null}
              Look up
            </button>
          </div>
          {join.invite ? (
            <span className="ftux-status" data-tone="ok" role="status">
              <CheckIcon size={10} />
              Joined {join.invite.host} · {join.invite.payRange}
            </span>
          ) : lookup.status === "error" ? (
            <span className="ftux-status" data-tone="error" role="alert">
              {lookup.error}
            </span>
          ) : null}
        </form>

        {showPasskey ? (
          <PasskeyCard passkey={join.passkey} onCreate={onCreatePasskey} />
        ) : null}

        <NearAiCard
          signedIn={join.nearAi}
          state={nearAi}
          onSignIn={onSignInNearAi}
        />

        {notice ? (
          <p className="ftux-well" role="status">
            {notice}
          </p>
        ) : null}
        <p className="ftux-well">
          Connecting or creating an account doesn't authorize any data sharing.
        </p>
      </div>
      <div className={`ftux-footer${hasAccount ? "" : " ftux-footer-split"}`}>
        {hasAccount ? null : (
          <span className="ftux-card-text ftux-muted" id="ftux-skip-note">
            Skipping sets up watching only. Contributing needs a near.ai
            account; sign in any time.
          </span>
        )}
        <button
          type="button"
          className="ftux-btn ftux-btn-primary"
          aria-describedby={hasAccount ? undefined : "ftux-skip-note"}
          onClick={onNext}
        >
          {hasAccount ? "Continue" : "Skip: watch only"}
        </button>
      </div>
    </>
  );
}

function PasskeyCard({
  passkey,
  onCreate,
}: {
  passkey: JoinState["passkey"];
  onCreate: () => void;
}) {
  return (
    <div className="ftux-card" data-done={passkey !== null}>
      <div className="ftux-card-row">
        <span style={{ display: "flex", flexDirection: "column" }}>
          <span className="ftux-section-label">Sign in with a passkey</span>
          <span className="ftux-card-text">
            {passkey
              ? `“${passkey.name}” is ready. Connect it to near.ai any time.`
              : "Create a passkey that can be connected later."}
          </span>
        </span>
        {passkey ? (
          <span className="ftux-status" data-tone="ok">
            <CheckIcon size={10} />
            Done
          </span>
        ) : (
          <button type="button" className="ftux-btn" onClick={onCreate}>
            Create passkey
          </button>
        )}
      </div>
    </div>
  );
}

function NearAiCard({
  signedIn,
  state,
  onSignIn,
}: {
  signedIn: boolean;
  state: AsyncState;
  onSignIn: () => void;
}) {
  return (
    <div className="ftux-card" data-done={signedIn}>
      <div className="ftux-card-row">
        <span style={{ display: "flex", flexDirection: "column" }}>
          <span className="ftux-section-label">Sign in with near.ai</span>
          <span className="ftux-card-text">
            Use the login you already have. Credits land in that account.
          </span>
        </span>
        {signedIn ? (
          <span className="ftux-status" data-tone="ok">
            <CheckIcon size={10} />
            Signed in
          </span>
        ) : (
          <button
            type="button"
            className="ftux-btn"
            disabled={state.status === "busy"}
            onClick={onSignIn}
          >
            {state.status === "busy" ? <Spinner /> : null}
            Sign in <ExternalIcon />
          </button>
        )}
      </div>
      {state.status === "error" ? (
        <span className="ftux-status" data-tone="error" role="alert">
          {state.error}
        </span>
      ) : null}
    </div>
  );
}
