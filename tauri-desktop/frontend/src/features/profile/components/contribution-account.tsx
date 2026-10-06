import { useEffect, useRef, useState } from "react";
import { scopedContributionLine, nextInviteAttempt, type InviteAttempt } from "../account-contribution";
import { invokeTauri } from "../../../lib/tauri/core-api";

type ContributionCopy = {
  heading: string;
  refresh: string;
  refreshAction: string;
  inviteCode: string;
  redeemAction: string;
  checking: string;
  unavailable: string;
  pendingCredit: string;
};

// DRAFT, NEEDS APPROVAL. The one sentence this shell owns: the core's own
// words are what failed to arrive, so it cannot supply this one.
const COPY_UNREADABLE =
  "Account contribution status cannot be shown, because this build could not read its wording.";

export function ContributionAccount({ scope }: { scope: string | null }) {
  const [copy, setCopy] = useState<ContributionCopy | null>(null);
  const [copyFailed, setCopyFailed] = useState(false);
  const [line, setLine] = useState("");
  useEffect(() => {
    let active = true;
    void invokeTauri("account_contribution_copy").then(value => {
      if (!active) return;
      const words = value as ContributionCopy;
      setCopy(words);
      setLine(words.refresh);
    }).catch(() => {
      // A refusal here (an ACL miss, a missing command) used to leave an
      // empty card with dead buttons and nothing saying why.
      if (active) setCopyFailed(true);
    });
    return () => { active = false; };
  }, []);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const attempt = useRef<InviteAttempt | null>(null);
  const perform = async (redeem: boolean) => {
    if (busy || !copy || !scope) return;
    setBusy(true);
    setLine(copy.checking);
    try {
      if (redeem) attempt.current = nextInviteAttempt(attempt.current, code, () => crypto.randomUUID(), scope);
      const result = await invokeTauri(redeem ? "account_invite_redeem" : "account_contribution_status", redeem ? {
        inviteCode: code, idempotencyKey: attempt.current!.key, accountScope: scope,
      } : { accountScope: scope });
      setLine(scopedContributionLine(result, scope));
      if (redeem) { setCode(""); attempt.current = null; }
    } catch { setLine(copy.unavailable); }
    finally { setBusy(false); }
  };
  if (!copy) {
    return <section className="my-6 rounded border p-4">
      <p role={copyFailed ? "alert" : "status"}>{copyFailed ? COPY_UNREADABLE : ""}</p>
    </section>;
  }
  return <section className="my-6 rounded border p-4" aria-label={copy.heading}>
    <h2>{copy.heading}</h2>
    <p role="status">{line}</p>
    <button type="button" disabled={busy || !scope} onClick={() => void perform(false)}>{copy.refreshAction}</button>
    <label className="block mt-3">{copy.inviteCode} <input type="password" autoComplete="off" value={code} disabled={busy || !scope} onChange={e => setCode(e.target.value)} /></label>
    <button type="button" disabled={busy || !scope || !code.trim()} onClick={() => void perform(true)}>{copy.redeemAction}</button>
    <p>{copy.pendingCredit}</p>
  </section>;
}
