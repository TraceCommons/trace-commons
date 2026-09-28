import assert from "node:assert/strict";
import { test } from "node:test";
import {
  inferenceView,
  installTarget,
  needsAccountSignIn,
  parseCurrentConnection,
  parseOffers,
  parseSelectResult,
  selectableOffers,
} from "./inference-connection.ts";
import { parseInferenceConnectionCopy } from "../../lib/tauri/inference-connection-copy.ts";

const digest = "a".repeat(64);
const offer = {
  offer_id: "near-ai",
  revision: digest,
  provider_id: "near-ai",
  disclosure_version: "inference-connection-disclosure-v1",
  config_digest: digest,
  disclosure: "WORDS",
};

const selection = {
  connection_id: "3f1c0000-0000-4000-8000-000000000001",
  state_version: 2,
  offer_id: "near-ai",
  revision: digest,
  config_digest: digest,
  disclosure_version: "inference-connection-disclosure-v1",
  reselection_required: false,
};

const current = (over = {}) =>
  parseCurrentConnection({
    selection: null,
    installed_on_this_device: false,
    pending_install: false,
    revocation_applied: false,
    ...over,
  });

test("offers are refused whole when any one is malformed", () => {
  assert.deepEqual(parseOffers({ offers: [offer] }), [offer]);
  assert.deepEqual(parseOffers({ offers: [] }), []);
  for (const bad of [
    null,
    { offers: 3 },
    { offers: [{ ...offer, offer_id: 7 }] },
    { offers: [offer, { ...offer, disclosure: 4 }] },
    { offers: [{ ...offer, disclosure: undefined }] },
  ]) {
    assert.throws(() => parseOffers(bad), /Invalid/);
  }
});

test("an offer the core cannot describe is never offered", () => {
  const unknown = { ...offer, offer_id: "other", disclosure: null };
  assert.deepEqual(selectableOffers([offer, unknown]), [offer]);
});

test("the current selection is parsed or refused", () => {
  assert.equal(current().selection, null);
  assert.deepEqual(current({ selection }).selection, selection);
  assert.throws(() => parseCurrentConnection({ selection: null }), /Invalid/);
  assert.throws(
    () => current({ selection: { ...selection, state_version: "2" } }),
    /Invalid/,
  );
});

test("a select result carries what install needs", () => {
  const result = parseSelectResult({
    selected: true,
    connection_id: selection.connection_id,
    state_version: 2,
    offer_id: "near-ai",
    revision: digest,
    config_digest: digest,
    disclosure_version: offer.disclosure_version,
    receipt_endpoint_offered: false,
    install_required: true,
    previous_witness_removed: true,
  });
  assert.deepEqual(result, {
    connection_id: selection.connection_id,
    config_digest: digest,
    previous_witness_removed: true,
  });
  assert.throws(() => parseSelectResult({ selected: false }), /Invalid/);
});

test("the daemon's missing account session is a sign-in, not a failure", () => {
  assert.equal(
    needsAccountSignIn(new Error("unavailable: account-session-required")),
    true,
  );
  assert.equal(needsAccountSignIn("unavailable: account-session-required"), true);
  assert.equal(
    needsAccountSignIn(new Error("unavailable: inference-connection-unavailable")),
    false,
  );
});

test("install is offered only for a held selection that is still current", () => {
  assert.equal(installTarget(current(), null), null);
  // Held from an earlier select on this device.
  assert.deepEqual(
    installTarget(current({ selection, pending_install: true }), null),
    { connection_id: selection.connection_id, config_digest: digest },
  );
  // Just selected in this step.
  const justSelected = {
    connection_id: selection.connection_id,
    config_digest: digest,
    previous_witness_removed: false,
  };
  assert.deepEqual(installTarget(current({ selection }), justSelected), {
    connection_id: selection.connection_id,
    config_digest: digest,
  });
  // Retired, or already installed here: nothing to install.
  assert.equal(
    installTarget(
      current({
        selection: { ...selection, reselection_required: true },
        pending_install: true,
      }),
      null,
    ),
    null,
  );
  assert.equal(
    installTarget(
      current({ selection, installed_on_this_device: true }),
      justSelected,
    ),
    null,
  );
  // A selection on another device is never installed here.
  assert.equal(installTarget(current({ selection }), null), null);
});

test("the step's view follows the daemon's answers", () => {
  const base = {
    signedIn: true,
    offers: [offer],
    current: current(),
    justSelected: null,
    loading: false,
  };
  assert.equal(inferenceView({ ...base, loading: true }).kind, "loading");
  assert.equal(inferenceView({ ...base, signedIn: false }).kind, "sign_in");
  assert.equal(inferenceView({ ...base, signedIn: null }).kind, "loading");
  assert.equal(inferenceView({ ...base, offers: [] }).kind, "none");
  const choose = inferenceView(base);
  assert.equal(choose.kind, "choose");
  assert.equal(choose.otherDevice, false);
  assert.equal(choose.reselect, false);
  // Selected on another device: say so before selecting here.
  const elsewhere = inferenceView({ ...base, current: current({ selection }) });
  assert.equal(elsewhere.kind, "choose");
  assert.equal(elsewhere.otherDevice, true);
  assert.equal(elsewhere.expectedVersion, 2);
  // Retired revision: choose again, the installed witness stays meanwhile.
  const retired = inferenceView({
    ...base,
    current: current({
      selection: { ...selection, reselection_required: true },
      installed_on_this_device: true,
    }),
  });
  assert.equal(retired.kind, "choose");
  assert.equal(retired.reselect, true);
  assert.equal(retired.otherDevice, false);
  assert.equal(
    inferenceView({
      ...base,
      current: current({ selection, installed_on_this_device: true }),
    }).kind,
    "installed",
  );
  assert.equal(
    inferenceView({
      ...base,
      current: current({ selection, pending_install: true }),
    }).kind,
    "install",
  );
  // Offers that the core cannot describe count as none.
  assert.equal(
    inferenceView({ ...base, offers: [{ ...offer, disclosure: null }] }).kind,
    "none",
  );
});

test("the step's copy is refused unless every sentence is present", () => {
  const copy = {
    why: "a",
    sign_in: "a",
    sign_in_failed: "a",
    load_failed: "a",
    none_offered: "a",
    grants_nothing: "a",
    one_device: "a",
    other_device: "a",
    install: "a",
    installed: "a",
    previous_removed: "a",
    reselect: "a",
    select_failed: "a",
    install_failed: "a",
    disconnect: "a",
    disconnect_pending: "a",
    unknown_disclosure: "a",
  };
  assert.deepEqual(parseInferenceConnectionCopy(copy), copy);
  for (const key of Object.keys(copy)) {
    assert.throws(
      () => parseInferenceConnectionCopy({ ...copy, [key]: "" }),
      /Invalid/,
    );
  }
});
