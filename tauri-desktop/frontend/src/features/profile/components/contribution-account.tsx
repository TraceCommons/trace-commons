import { useEffect, useRef, useState } from "react";
import { scopedContributionLine, nextInviteAttempt, type InviteAttempt } from "../account-contribution";
import { invokeTauri } from "../../../lib/tauri/core-api";

export function ContributionAccount({ scope }: { scope: string | null }) {
  const [copy, setCopy] = useState<{refresh: string; checking: string; unavailable: string; pendingCredit: string} | null>(null);
  const [line, setLine] = useState("");
  useEffect(() => {
    let active = true;
    void invokeTauri("account_contribution_copy").then(value => {
      if (!active) return;
      const words = value as {refresh: string; checking: string; unavailable: string; pendingCredit: string};
      setCopy(words);
      setLine(words.refresh);
    }).catch(() => {});
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
  return <section className="my-6 rounded border p-4" aria-label="Account contributions">
    <h2>Account contributions</h2>
    <p role="status">{line}</p>
    <button type="button" disabled={busy || !copy || !scope} onClick={() => void perform(false)}>Refresh status</button>
    <label className="block mt-3">Invite code <input type="password" autoComplete="off" value={code} disabled={busy || !copy || !scope} onChange={e => setCode(e.target.value)} /></label>
    <button type="button" disabled={busy || !copy || !scope || !code.trim()} onClick={() => void perform(true)}>Redeem invite</button>
    <p>{copy?.pendingCredit}</p>
  </section>;
}
