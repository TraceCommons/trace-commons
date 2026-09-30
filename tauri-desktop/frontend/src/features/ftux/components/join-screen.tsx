import type { ReactNode } from "react";
import {
  ButtonPrimary,
  Card,
  GlassButton,
  StatusLabel,
} from "../../../design-system";
import { MOCK_INVITE_PLACEHOLDER } from "../api/ftux-mock-data";
import type { JoinState } from "../types";
import {
  Notice,
  ScreenBody,
  ScreenFooter,
  ScreenTitle,
  StatusLine,
} from "./ftux-frame";
import { ExternalIcon, Spinner } from "./icons";

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
      <p className="m-0 tc-text-secondary">
        Start with an invite link, sign-up or sign-in with an existing account,
        or just click "Skip".{" "}
        <b className="tc-text-primary">
          You'll be able to setup or connect your account later to receive
          credits and manage access to the near.ai ecosystem.
        </b>
      </p>
      <ScreenBody>
        <InviteCard
          invite={join.invite}
          draft={inviteDraft}
          state={lookup}
          onDraft={onInviteDraft}
          onLookup={onLookup}
        />
        {showPasskey ? (
          <PasskeyCard passkey={join.passkey} onCreate={onCreatePasskey} />
        ) : null}
        <NearAiCard
          signedIn={join.nearAi}
          state={nearAi}
          onSignIn={onSignInNearAi}
        />
        {notice ? (
          <Notice tone="ask" role="status">
            {notice}
          </Notice>
        ) : null}
        <Card quiet className="tc-text-secondary">
          Connecting or creating an account doesn't authorize any data sharing.
        </Card>
      </ScreenBody>
      <ScreenFooter
        noteId="ftux-skip-note"
        note={
          hasAccount
            ? undefined
            : "Skipping sets up watching only. Contributing needs a near.ai account; sign in any time."
        }
      >
        <ButtonPrimary
          aria-describedby={hasAccount ? undefined : "ftux-skip-note"}
          onClick={onNext}
        >
          {hasAccount ? "Continue" : "Skip: watch only"}
        </ButtonPrimary>
      </ScreenFooter>
    </>
  );
}

function InviteCard({
  invite,
  draft,
  state,
  onDraft,
  onLookup,
}: {
  invite: JoinState["invite"];
  draft: string;
  state: AsyncState;
  onDraft: (value: string) => void;
  onLookup: () => void;
}) {
  return (
    <Card className="tc-stack tc-stack--tight">
      <form
        className="tc-field"
        onSubmit={(event) => {
          event.preventDefault();
          onLookup();
        }}
      >
        <label className="tc-eyebrow" htmlFor="ftux-invite">
          Invite link
        </label>
        <div className="ftux-row">
          <input
            id="ftux-invite"
            className="tc-input tc-mono ftux-grow"
            value={draft}
            placeholder={`${MOCK_INVITE_PLACEHOLDER}…`}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => onDraft(event.target.value)}
          />
          <GlassButton
            type="submit"
            disabled={state.status === "busy" || !draft.trim()}
          >
            {state.status === "busy" ? <Spinner /> : null}
            Look up
          </GlassButton>
        </div>
      </form>
      {invite ? (
        <StatusLine tone="ok">
          Joined {invite.host} · {invite.payRange}
        </StatusLine>
      ) : state.status === "error" ? (
        <StatusLine tone="error">{state.error}</StatusLine>
      ) : null}
    </Card>
  );
}

function AccountCard({
  eyebrow,
  text,
  done,
  doneLabel,
  children,
}: {
  eyebrow: string;
  text: string;
  done: boolean;
  doneLabel: string;
  children: ReactNode;
}) {
  return (
    <Card>
      <div className="ftux-row ftux-row--between">
        <span className="tc-stack tc-stack--tight ftux-gap-2">
          <span className="tc-eyebrow">{eyebrow}</span>
          <span className="tc-label tc-text-secondary">{text}</span>
        </span>
        {done ? <StatusLabel tone="on">{doneLabel}</StatusLabel> : children}
      </div>
    </Card>
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
    <AccountCard
      eyebrow="Sign in with a passkey"
      text={
        passkey
          ? `“${passkey.name}” is ready. Connect it to near.ai any time.`
          : "Create a passkey that can be connected later."
      }
      done={passkey !== null}
      doneLabel="Done"
    >
      <GlassButton onClick={onCreate}>Create passkey</GlassButton>
    </AccountCard>
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
    <>
      <AccountCard
        eyebrow="Sign in with near.ai"
        text="Use the login you already have. Credits land in that account."
        done={signedIn}
        doneLabel="Signed in"
      >
        <GlassButton disabled={state.status === "busy"} onClick={onSignIn}>
          {state.status === "busy" ? <Spinner /> : null}
          Sign in <ExternalIcon />
        </GlassButton>
      </AccountCard>
      {state.status === "error" ? (
        <StatusLine tone="error">{state.error}</StatusLine>
      ) : null}
    </>
  );
}
