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
}) {
  const joined = join.invite !== null || join.passkey !== null || join.nearAi;
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

        <div className="ftux-card" data-done={join.passkey !== null}>
          <div className="ftux-card-row">
            <span style={{ display: "flex", flexDirection: "column" }}>
              <span className="ftux-section-label">Sign in with a passkey</span>
              <span className="ftux-card-text">
                {join.passkey
                  ? `“${join.passkey.name}” is ready. Connect it to near.ai any time.`
                  : "Create a passkey that can be connected later."}
              </span>
            </span>
            {join.passkey ? (
              <span className="ftux-status" data-tone="ok">
                <CheckIcon size={10} />
                Done
              </span>
            ) : (
              <button
                type="button"
                className="ftux-btn"
                onClick={onCreatePasskey}
              >
                Create passkey
              </button>
            )}
          </div>
        </div>

        <div className="ftux-card" data-done={join.nearAi}>
          <div className="ftux-card-row">
            <span style={{ display: "flex", flexDirection: "column" }}>
              <span className="ftux-section-label">Sign in with near.ai</span>
              <span className="ftux-card-text">
                Use the login you already have. Credits land in that account.
              </span>
            </span>
            {join.nearAi ? (
              <span className="ftux-status" data-tone="ok">
                <CheckIcon size={10} />
                Signed in
              </span>
            ) : (
              <button
                type="button"
                className="ftux-btn"
                disabled={nearAi.status === "busy"}
                onClick={onSignInNearAi}
              >
                {nearAi.status === "busy" ? <Spinner /> : null}
                Sign in <ExternalIcon />
              </button>
            )}
          </div>
          {nearAi.status === "error" ? (
            <span className="ftux-status" data-tone="error" role="alert">
              {nearAi.error}
            </span>
          ) : null}
        </div>

        {notice ? (
          <p className="ftux-well" role="status">
            {notice}
          </p>
        ) : null}
        <p className="ftux-well">
          Connecting or creating an account doesn't authorize any data sharing.
        </p>
      </div>
      <div className="ftux-footer">
        <button
          type="button"
          className="ftux-btn ftux-btn-primary"
          onClick={onNext}
        >
          {joined ? "Continue" : "Skip"}
        </button>
      </div>
    </>
  );
}
