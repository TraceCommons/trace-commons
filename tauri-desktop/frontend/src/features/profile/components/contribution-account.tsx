import { useEffect, useRef, useState } from "react";
import { contributionLine, nextInviteAttempt, type InviteAttempt } from "../account-contribution";
import { invokeTauri } from "../../../lib/tauri/core-api";

export function ContributionAccount() {
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
    if (busy || !copy) return;
    setBusy(true);
    setLine(copy.checking);
    try {
      if (redeem) attempt.current = nextInviteAttempt(attempt.current, code, () => crypto.randomUUID());
      const result = await invokeTauri(redeem ? "account_invite_redeem" : "account_contribution_status", redeem ? {
        inviteCode: code, idempotencyKey: attempt.current!.key,
      } : {});
      setLine(contributionLine(result));
      if (redeem) { setCode(""); attempt.current = null; }
    } catch { setLine(copy.unavailable); }
    finally { setBusy(false); }
  };
  return <section className="my-6 rounded border p-4" aria-label="Account contributions">
    <h2>Account contributions</h2>
    <p role="status">{line}</p>
    <button type="button" disabled={busy || !copy} onClick={() => void perform(false)}>Refresh status</button>
    <label className="block mt-3">Invite code <input type="password" autoComplete="off" value={code} disabled={busy || !copy} onChange={e => setCode(e.target.value)} /></label>
    <button type="button" disabled={busy || !copy || !code.trim()} onClick={() => void perform(true)}>Redeem invite</button>
    <p>{copy?.pendingCredit}</p>
  </section>;
}
