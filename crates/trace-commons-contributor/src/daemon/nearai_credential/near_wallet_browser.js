// INTEGRATION: load after the pinned HOTConnect IIFE on the IPv4 loopback
// callback page. Required DOM: #challenge (application/json), #connect and
// #cancel buttons, and #status. Challenge JSON supplies message, recipient,
// nonce[32], expires_at_ms, pinned manifest {version,wallets}, cspNonce, and
// canonical copy {choose,waiting,received,refused,expired,cancelled,unavailable}.
// Render #status initially with canonical refused copy for invalid JSON.
// The initial URL fragment is the original 43-character ceremony state.
// Rust verifies the deposited signature; Cloud verifies account ownership.
// No Cloud tokens, callbackUrl signing parameter, or transaction API is used.
// SDK 0.11.4 has no public cancellation API: terminal states discard late
// results and repeat local wallet-storage cleanup when pending work settles.
(() => {
  "use strict";

  const state = window.location.hash.slice(1);
  let fragmentRemoved = false;
  try {
    window.history.replaceState(null, "", window.location.pathname + window.location.search);
    fragmentRemoved = true;
  } catch (_) {
    // A fragment that cannot be removed must not proceed to wallet code.
  }

  const connectButton = document.getElementById("connect");
  const cancelButton = document.getElementById("cancel");
  const status = document.getElementById("status");
  const challengeElement = document.getElementById("challenge");
  if (!connectButton || !cancelButton || !status) return;
  connectButton.disabled = true;
  cancelButton.disabled = true;

  const copyKeys = ["choose", "waiting", "received", "refused", "expired", "cancelled", "unavailable"];
  const record = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
  let challenge;
  try {
    if (!fragmentRemoved || !/^[A-Za-z0-9_-]{43}$/.test(state)
        || window.location.protocol !== "http:" || window.location.hostname !== "127.0.0.1"
        || !window.location.port || window.location.pathname !== "/near-ai/near-wallet/callback"
        || window.location.search || window.top !== window
        || !challengeElement || challengeElement.type !== "application/json"
        || challengeElement.textContent.length > 256 * 1024) throw new Error();
    challenge = JSON.parse(challengeElement.textContent);
    if (!record(challenge) || challenge.message !== "Sign in to NEAR AI Cloud"
        || challenge.recipient !== "cloud.near.ai"
        || !Array.isArray(challenge.nonce) || challenge.nonce.length !== 32
        || !challenge.nonce.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)
        || !Number.isSafeInteger(challenge.expires_at_ms)
        || challenge.expires_at_ms > Date.now() + 300000
        || typeof challenge.cspNonce !== "string" || !/^[A-Za-z0-9+/_=-]{1,128}$/.test(challenge.cspNonce)
        || !record(challenge.copy) || !copyKeys.every((key) => typeof challenge.copy[key] === "string"
          && challenge.copy[key].length > 0 && challenge.copy[key].length <= 1024)
        || !record(challenge.manifest) || typeof challenge.manifest.version !== "string"
        || challenge.manifest.version.length === 0 || challenge.manifest.version.length > 64
        || !Array.isArray(challenge.manifest.wallets) || challenge.manifest.wallets.length > 64
        || !challenge.manifest.wallets.every((wallet) => record(wallet)
          && typeof wallet.id === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(wallet.id))) {
      throw new Error();
    }
  } catch (_) {
    if (record(challenge?.copy) && typeof challenge.copy.refused === "string"
        && challenge.copy.refused.length <= 1024) status.textContent = challenge.copy.refused;
    return;
  }

  const text = challenge.copy;
  const walletPrefixes = challenge.manifest.wallets.map((wallet) => wallet.id + ":");
  const selection = new Map();
  let phase = "ready";
  let connector;
  let deadlineTimer;
  let activeRequest;

  const walletUiClosed = () => phase !== "ready" && phase !== "wallet";
  function removeWalletPopups() {
    // Both chooser and iframe wrappers use this stable class in SDK 0.11.4.
    document.body.querySelectorAll(":scope > .hot-connector-popup").forEach((popup) => popup.remove());
  }
  const popupObserver = new MutationObserver(() => {
    if (walletUiClosed()) removeWalletPopups();
  });
  popupObserver.observe(document.body, { childList: true });
  // Install before the SDK's listeners. Removing an iframe does not discard
  // messages already queued to its parent's storage or popup handlers.
  window.addEventListener("message", (event) => {
    if (walletUiClosed() || event.data?.type === "near-wallet-injected") event.stopImmediatePropagation();
  }, true);
  // The pinned manifest owns wallet IDs. SDK discovery events otherwise let
  // an unrelated window or injected script replace a reviewed executor.
  window.addEventListener("near-wallet-injected", (event) => event.stopImmediatePropagation(), true);

  // SandboxExecutor uses localStorage directly under its manifest ID. Only
  // those wallet namespaces on this ephemeral origin are removed. Calling
  // disconnect/signOut would also change the remote wallet's session.
  function clearLocalWalletState() {
    selection.clear();
    try {
      for (let index = window.localStorage.length - 1; index >= 0; index -= 1) {
        const key = window.localStorage.key(index);
        if (key && walletPrefixes.some((prefix) => key.startsWith(prefix))) {
          window.localStorage.removeItem(key);
        }
      }
    } catch (_) {
      // Storage can be denied by the browser; no values enter diagnostics.
    }
  }

  function end(message, disposed = false) {
    phase = disposed ? "disposed" : "terminal";
    connectButton.disabled = true;
    cancelButton.disabled = true;
    window.clearTimeout(deadlineTimer);
    activeRequest?.abort();
    removeWalletPopups();
    clearLocalWalletState();
    if (!disposed) status.textContent = message;
  }

  function stillValid() {
    if (phase === "terminal" || phase === "disposed") return false;
    if (Date.now() >= challenge.expires_at_ms) {
      end(text.expired);
      return false;
    }
    return true;
  }

  function clearExecutorCache() {
    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (success) => {
        if (settled) return;
        settled = true;
        window.clearTimeout(timer);
        if (success) resolve();
        else reject(new Error());
      };
      const timer = window.setTimeout(() => finish(false), 3000);
      try {
        // Pinned SDK 0.11.4 owns this database. A fresh ceremony must not
        // execute cached code from a previous owner of this loopback port.
        const request = window.indexedDB.deleteDatabase("hot-connector");
        request.onsuccess = () => finish(true);
        request.onerror = () => finish(false);
        request.onblocked = () => finish(false);
      } catch (_) {
        finish(false);
      }
    });
  }

  function encodeResult(result) {
    if (!record(result) || Object.keys(result).length !== 3
        || typeof result.accountId !== "string" || result.accountId.length < 2
        || result.accountId.length > 64 || !/^[a-z0-9]+(?:[._-][a-z0-9]+)*$/.test(result.accountId)
        || typeof result.publicKey !== "string"
        || !/^ed25519:[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(result.publicKey)
        || typeof result.signature !== "string" || !/^[A-Za-z0-9+/]{86}==$/.test(result.signature)) {
      throw new Error();
    }
    const signature = window.atob(result.signature);
    if (signature.length !== 64 || window.btoa(signature) !== result.signature) throw new Error();
    return new URLSearchParams({
      accountId: result.accountId,
      publicKey: result.publicKey,
      signature: result.signature,
      state,
    });
  }

  async function deposit(body, cancelled) {
    if ((phase !== "ready" && phase !== "wallet") || !stillValid()) return;
    phase = "deposit";
    removeWalletPopups();
    connectButton.disabled = true;
    cancelButton.disabled = true;
    status.textContent = text.waiting;
    activeRequest?.abort();
    activeRequest = new AbortController();
    try {
      const response = await window.fetch("/near-ai/near-wallet/result", {
        method: "POST",
        mode: "same-origin",
        credentials: "omit",
        redirect: "error",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: body.toString(),
        signal: activeRequest.signal,
      });
      // Neither a successful receipt nor an error body carries browser data.
      if (response.body) void response.body.cancel().catch(() => {});
      if (phase !== "deposit" || !stillValid()) return;
      end(response.ok ? (cancelled ? text.cancelled : text.received) : text.refused);
    } catch (_) {
      if (phase === "deposit" && stillValid()) end(text.refused);
    }
  }

  connectButton.addEventListener("click", async () => {
    if (phase !== "ready" || !stillValid()) return;
    phase = "wallet";
    connectButton.disabled = true;
    status.textContent = text.waiting;
    try {
      if (!connector) {
        try {
          await clearExecutorCache();
        } catch (_) {
          // IndexedDB deletion cannot be cancelled. Keep this ceremony
          // terminal so a late deletion cannot overlap a retry's SDK.
          if (phase === "wallet" && stillValid()) end(text.refused);
          return;
        }
        if (phase !== "wallet" || !stillValid()) return;
        connector = new window.HOTConnect.NearConnector({
          network: "mainnet",
          features: { signMessage: true, signInWithoutAddKey: true },
          excludedWallets: ["mynearwallet"],
          autoConnect: false,
          footerBranding: null,
          manifest: challenge.manifest,
          cspNonce: challenge.cspNonce,
          storage: {
            get: async (key) => selection.get(key) ?? null,
            set: async (key, value) => {
              if (phase !== "terminal" && phase !== "disposed") selection.set(key, value);
            },
            remove: async (key) => { selection.delete(key); },
          },
        });
      }
      const walletId = await connector.selectWallet({
        features: { signMessage: true, signInWithoutAddKey: true },
      });
      if (phase !== "wallet" || !stillValid()) return;
      const manifest = challenge.manifest.wallets.find((wallet) => wallet.id === walletId);
      if (!manifest || walletId === "mynearwallet" || typeof manifest.executor !== "string"
          || manifest.executor.length > 2048) throw new Error();
      const executor = new URL(manifest.executor, window.location.origin);
      if (executor.origin !== window.location.origin || executor.username || executor.password
          || executor.pathname !== "/near-ai/near-wallet/assets/" + walletId + ".js"
          || executor.search || executor.hash) throw new Error();
      activeRequest = new AbortController();
      // The SDK's own adapter fetch ignores HTTP status. Confirm the local
      // server has verified and cached this pinned executor before connecting.
      const asset = await window.fetch(executor.href, {
        mode: "same-origin", credentials: "omit", redirect: "error", signal: activeRequest.signal,
      });
      if (asset.body) void asset.body.cancel().catch(() => {});
      if (phase !== "wallet" || !stillValid()) return;
      if (!asset.ok) throw new Error();
      const wallet = await connector.connect({ walletId });
      if (phase !== "wallet" || !stillValid()) return;
      if (!wallet || typeof wallet.signMessage !== "function"
          || wallet.manifest?.id === "mynearwallet"
          || wallet.manifest?.features?.signMessage !== true
          || wallet.manifest?.features?.signInWithoutAddKey !== true) throw new Error();
      const result = await wallet.signMessage({
        message: challenge.message,
        recipient: challenge.recipient,
        nonce: new Uint8Array(challenge.nonce),
        network: "mainnet",
      });
      if (phase !== "wallet" || !stillValid()) return;
      await deposit(encodeResult(result), false);
    } catch (_) {
      if (phase === "wallet" && stillValid()) {
        phase = "ready";
        connectButton.disabled = false;
        status.textContent = text.unavailable;
      }
    } finally {
      if (phase === "terminal" || phase === "disposed") clearLocalWalletState();
    }
  });

  cancelButton.addEventListener("click", () => {
    if (!stillValid()) return;
    void deposit(new URLSearchParams({ state, error: "cancelled" }), true);
  });
  window.addEventListener("pagehide", () => end("", true), { once: true });
  if (!stillValid()) return;
  deadlineTimer = window.setTimeout(() => end(text.expired), challenge.expires_at_ms - Date.now());
  connectButton.disabled = false;
  cancelButton.disabled = false;
  status.textContent = text.choose;
})();
