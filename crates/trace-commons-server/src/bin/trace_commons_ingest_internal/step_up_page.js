// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The browser passkey step-up page (Z2 S7). Served inline by
// `step_up_page.rs`, which pins this exact text by its SHA-256 in the page's
// Content-Security-Policy; any edit here changes the hash, and the CSP is
// recomputed from this file at startup.
//
// It calls only the existing routes in ROUTES and adds no API. Every string
// shown to the person comes from the copy table in `step_up_page.rs` (read
// here through `t`). Text goes into the page through `textContent` only.
'use strict';
(() => {
  const ROUTES = {
    loginStart: '/account/passkey/login/start',
    loginFinish: '/account/passkey/login/finish',
    passkeys: '/v1/account/passkeys',
    passkey: '/v1/account/passkeys/{credential_id}',
    registerStart: '/v1/account/passkeys/register/start',
    registerFinish: '/v1/account/passkeys/register/finish',
    nearIdentities: '/v1/account/near-identities',
    payout: '/v1/account/near-identities/{public_key}/payout',
    logout: '/v1/account/logout',
  };
  const SECTIONS = ['add-passkey', 'remove-passkey', 'change-payout'];

  const $ = (id) => document.getElementById(id);
  const copy = JSON.parse($('tc-copy').textContent);
  const t = (key) => copy[key] || '';
  const statusLine = $('status');
  const say = (key) => {
    statusLine.textContent = t(key);
  };
  const action = document.body.dataset.action || '';

  const toBytes = (text) => {
    const b64 = text.replace(/-/g, '+').replace(/_/g, String.fromCharCode(47));
    const padded = b64 + '='.repeat((4 - (b64.length % 4)) % 4);
    return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
  };
  const toText = (buffer) => {
    let binary = '';
    for (const byte of new Uint8Array(buffer)) binary += String.fromCharCode(byte);
    return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  };
  const withId = (template, value) =>
    template.replace(/\{[a-z_]+\}/, encodeURIComponent(value));

  const send = (method, url, body, extra) =>
    fetch(url, {
      method,
      credentials: 'same-origin',
      cache: 'no-store',
      referrerPolicy: 'no-referrer',
      headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      ...(extra || {}),
    });

  // A failed call leaves the status line saying so: refused by the
  // strong-session gate, or anything else.
  const failed = (response) => {
    say(response && response.status === 403 ? 'action_refused' : 'action_failed');
  };

  const withIds = (list) => (list || []).map((c) => ({ ...c, id: toBytes(c.id) }));

  async function signIn() {
    const button = $('sign-in');
    button.disabled = true;
    say('status_signing_in');
    try {
      const start = await send('POST', ROUTES.loginStart);
      if (!start.ok) throw new Error('start');
      const options = (await start.json()).publicKey;
      options.challenge = toBytes(options.challenge);
      options.allowCredentials = withIds(options.allowCredentials);
      const credential = await navigator.credentials.get({ publicKey: options });
      if (!credential) throw new Error('cancelled');
      const r = credential.response;
      const finish = await send(
        'POST',
        ROUTES.loginFinish,
        {
          id: credential.id,
          rawId: toText(credential.rawId),
          type: credential.type,
          response: {
            authenticatorData: toText(r.authenticatorData),
            clientDataJSON: toText(r.clientDataJSON),
            signature: toText(r.signature),
            userHandle: r.userHandle ? toText(r.userHandle) : null,
          },
        },
        // Success is a 303 carrying the session cookie. Following it would
        // leave this page, so it is observed instead of followed.
        { redirect: 'manual' },
      );
      if (finish.type !== 'opaqueredirect' && finish.status !== 303) throw new Error('finish');
      signedIn();
    } catch (_) {
      say('status_sign_in_failed');
      button.disabled = false;
    }
  }

  function signedIn() {
    $('sign-in-section').hidden = true;
    const wanted = action ? [action] : SECTIONS;
    for (const id of wanted) $(id).hidden = false;
    $('finish').hidden = false;
    say('status_signed_in');
    if (wanted.includes('remove-passkey')) loadPasskeys();
    if (wanted.includes('change-payout')) loadPayout();
  }

  async function addPasskey() {
    const button = $('add-passkey-button');
    button.disabled = true;
    say('status_waiting');
    try {
      const start = await send('POST', ROUTES.registerStart);
      if (!start.ok) return failed(start);
      const options = (await start.json()).publicKey;
      options.challenge = toBytes(options.challenge);
      options.user = { ...options.user, id: toBytes(options.user.id) };
      options.excludeCredentials = withIds(options.excludeCredentials);
      const credential = await navigator.credentials.create({ publicKey: options });
      if (!credential) return failed();
      const body = {
        id: credential.id,
        rawId: toText(credential.rawId),
        type: credential.type,
        response: {
          attestationObject: toText(credential.response.attestationObject),
          clientDataJSON: toText(credential.response.clientDataJSON),
        },
      };
      const label = $('passkey-label').value.trim();
      if (label) body.label = label;
      const finish = await send('POST', ROUTES.registerFinish, body);
      if (!finish.ok) return failed(finish);
      $('passkey-label').value = '';
      say('action_add_done');
      if (!$('remove-passkey').hidden) loadPasskeys();
    } catch (_) {
      failed();
    } finally {
      button.disabled = false;
    }
  }

  function row(list, text, note, buttonKey, onClick) {
    const item = document.createElement('li');
    const name = document.createElement('span');
    name.textContent = text;
    item.append(name);
    if (note) {
      const small = document.createElement('small');
      small.textContent = note;
      item.append(' ', small);
    }
    if (buttonKey) {
      const button = document.createElement('button');
      button.type = 'button';
      button.textContent = t(buttonKey);
      button.addEventListener('click', () => onClick(button));
      item.append(' ', button);
    }
    list.append(item);
  }

  async function loadPasskeys() {
    const list = $('passkey-list');
    list.replaceChildren();
    try {
      const response = await send('GET', ROUTES.passkeys);
      if (!response.ok) return failed(response);
      const { passkeys } = await response.json();
      for (const p of passkeys || []) {
        row(
          list,
          p.label || t('action_remove_unnamed'),
          p.this_device ? t('action_remove_this_device') : '',
          'action_remove_button',
          (button) => removePasskey(button, p.credential_id),
        );
      }
    } catch (_) {
      failed();
    }
  }

  async function removePasskey(button, credentialId) {
    if (!window.confirm(t('action_remove_confirm'))) return;
    button.disabled = true;
    try {
      const response = await send('DELETE', withId(ROUTES.passkey, credentialId));
      if (!response.ok) return failed(response);
      say('action_remove_done');
      loadPasskeys();
    } catch (_) {
      failed();
    } finally {
      button.disabled = false;
    }
  }

  async function loadPayout() {
    const list = $('payout-list');
    list.replaceChildren();
    try {
      const response = await send('GET', ROUTES.nearIdentities);
      if (!response.ok) return failed(response);
      const identities = (await response.json()).near_identities || [];
      if (identities.length === 0) {
        row(list, t('action_payout_none'));
        return;
      }
      for (const n of identities) {
        if (n.is_payout) {
          row(list, n.near_account_id, t('action_payout_current'));
        } else {
          row(list, n.near_account_id, '', 'action_payout_button', (button) =>
            choosePayout(button, n.public_key),
          );
        }
      }
    } catch (_) {
      failed();
    }
  }

  async function choosePayout(button, publicKey) {
    button.disabled = true;
    try {
      const response = await send('PATCH', withId(ROUTES.payout, publicKey), { payout: true });
      if (!response.ok) return failed(response);
      say('action_payout_done');
      loadPayout();
    } catch (_) {
      failed();
    } finally {
      button.disabled = false;
    }
  }

  async function signOut() {
    try {
      await send('POST', ROUTES.logout);
    } catch (_) {
      // Signed out or not, the page stops offering changes.
    }
    for (const id of SECTIONS) $(id).hidden = true;
    $('finish').hidden = true;
    say('signed_out');
  }

  if (!window.PublicKeyCredential || !navigator.credentials) {
    say('status_unsupported');
    return;
  }
  $('sign-in').disabled = false;
  $('sign-in').addEventListener('click', signIn);
  $('add-passkey-button').addEventListener('click', addPasskey);
  $('sign-out').addEventListener('click', signOut);
})();
