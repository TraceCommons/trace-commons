import assert from "node:assert/strict";
import { test } from "node:test";
import {
  parseCertificateDetail,
  parseRouteDisclosure,
} from "./route-disclosure.ts";

// Stand-ins, not the core's sentences: the shell renders whichever blocks
// the core sent, so the words do not matter here, only which are present.
const session = {
  heading: "SESSION",
  before_label: "BEFORE",
  before_line: "BEFORE LINE",
  after_label: "AFTER",
  after_line: "AFTER LINE",
};

const witnessFacts = {
  state: "pinned",
  url: "https://witness.invalid",
  signing_address: "0xab",
  pinned_measurements: ["mrtd=aa"],
  origin: "published_at_join",
};

const witnessCopy = {
  heading: "WITNESS",
  address_label: "ADDRESS",
  signing_label: "SIGNING",
  measurements_label: "PINS",
  check: "CHECK",
  classifier: "CLASSIFIER",
  origin: "ORIGIN",
};

const receipts = { endpoint_configured: false, check_attestation: false };

function witnessRoute() {
  return {
    facts: {
      route: "witness",
      witness: witnessFacts,
      local_filter: null,
      receipts,
      attested_bodies: false,
    },
    copy: {
      title: "TITLE",
      route: "RAW SEND",
      witness: witnessCopy,
      local_filter: null,
      receipts: "RECEIPTS",
      attested_bodies: null,
      session,
    },
  };
}

function localRoute() {
  return {
    facts: {
      route: "local",
      witness: null,
      local_filter: "near_ai",
      receipts,
      attested_bodies: false,
    },
    copy: {
      title: "TITLE",
      route: "LOCAL",
      witness: null,
      local_filter: "NEAR AI FILTER",
      receipts: null,
      attested_bodies: null,
      session,
    },
  };
}

test("the witness route carries the witness, its pins and its origin line", () => {
  const parsed = parseRouteDisclosure(witnessRoute());
  assert.equal(parsed.facts.route, "witness");
  assert.deepEqual(parsed.facts.witness?.pinned_measurements, ["mrtd=aa"]);
  assert.equal(parsed.facts.witness?.origin, "published_at_join");
  assert.equal(parsed.copy.witness?.origin, "ORIGIN");
  assert.equal(parsed.copy.witness?.classifier, "CLASSIFIER");
  assert.equal(parsed.copy.receipts, "RECEIPTS");
  assert.equal(parsed.copy.local_filter, null);
});

test("the local route carries its filter line and no witness", () => {
  const parsed = parseRouteDisclosure(localRoute());
  assert.equal(parsed.facts.local_filter, "near_ai");
  assert.equal(parsed.copy.local_filter, "NEAR AI FILTER");
  assert.equal(parsed.copy.witness, null);
});

test("words for a block the facts do not have are refused, not rendered", () => {
  const local = localRoute();
  local.copy.witness = witnessCopy;
  assert.throws(() => parseRouteDisclosure(local), /witness/);

  const witness = witnessRoute();
  witness.copy.local_filter = "A FILTER LINE";
  assert.throws(() => parseRouteDisclosure(witness), /local filter/);

  const refusing = witnessRoute();
  refusing.facts.route = "witness_refusing";
  refusing.facts.witness = { ...witnessFacts, state: "refusing_unpinned" };
  // A refusing witness is sent nothing, so no classifier or receipt line.
  assert.throws(() => parseRouteDisclosure(refusing), /classifier|receipt/);
  refusing.copy.witness = { ...witnessCopy, classifier: null };
  refusing.copy.receipts = null;
  assert.equal(parseRouteDisclosure(refusing).facts.route, "witness_refusing");
});

test("a route or origin this build does not know is refused", () => {
  const unknownRoute = witnessRoute();
  unknownRoute.facts.route = "somewhere_new";
  assert.throws(() => parseRouteDisclosure(unknownRoute), /route/);
  const unknownOrigin = witnessRoute();
  unknownOrigin.facts.witness = { ...witnessFacts, origin: "an_operator" };
  assert.throws(() => parseRouteDisclosure(unknownOrigin), /origin/);
  assert.throws(() => parseRouteDisclosure(null));
});

test("a certificate detail keeps the claims and the core's labels", () => {
  const parsed = parseCertificateDetail({
    detail: {
      state: "held",
      verification: "verified_at_review",
      witness_measurement: "mrtd=aa",
      signer: "0xab",
      residual_risk_verdict: "low",
    },
    copy: {
      heading: "HEADING",
      measurement_label: "MEASUREMENT",
      signer_label: "SIGNER",
      verified_at_review: "VERIFIED",
    },
  });
  assert.equal(parsed.detail.witness_measurement, "mrtd=aa");
  assert.equal(parsed.detail.signer, "0xab");
  assert.equal(parsed.copy.verified_at_review, "VERIFIED");
  // Only the one verification the daemon reports today is worded.
  assert.throws(() =>
    parseCertificateDetail({
      detail: {
        state: "held",
        verification: "verified_now",
        witness_measurement: "m",
        signer: "s",
      },
      copy: {
        heading: "H",
        measurement_label: "M",
        signer_label: "S",
        verified_at_review: "V",
      },
    }),
  );
});
