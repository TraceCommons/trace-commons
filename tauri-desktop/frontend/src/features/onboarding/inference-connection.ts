// Connecting inference (K12): parsers and the step's view, kept free of
// imports so the node test runner can load it directly. The flow is the
// daemon's (`docs/contributor-daemon-ipc-v1_1.md`, "Connecting inference"):
// offers, then select (records the choice on the account, installs
// nothing), then a separate confirmed install on this device.

/** One offer as the daemon validated it, plus the core's words for it. */
export type InferenceOffer = {
  offer_id: string;
  revision: string;
  provider_id: string;
  disclosure_version: string;
  config_digest: string;
  /** `null` when this build has no words for `disclosure_version`. */
  disclosure: string | null;
};

export type ConnectionSelection = {
  connection_id: string;
  state_version: number;
  offer_id: string;
  revision: string;
  config_digest: string;
  disclosure_version: string;
  reselection_required: boolean;
};

export type CurrentConnection = {
  selection: ConnectionSelection | null;
  installed_on_this_device: boolean;
  pending_install: boolean;
  revocation_applied: boolean;
};

export type SelectResult = {
  connection_id: string;
  config_digest: string;
  previous_witness_removed: boolean;
};

export type InstallTarget = { connection_id: string; config_digest: string };


type RecordValue = Record<string, unknown>;

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid ${label}`);
  }
  return value as RecordValue;
}

function text(value: RecordValue, key: string, label: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid ${label} field: ${key}`);
  }
  return field;
}

function flag(value: RecordValue, key: string, label: string): boolean {
  const field = value[key];
  if (typeof field !== "boolean") {
    throw new Error(`Invalid ${label} field: ${key}`);
  }
  return field;
}

/**
 * The whole list or nothing: one malformed offer refuses them all, as the
 * daemon does, rather than showing a subset.
 */
export function parseOffers(value: unknown): InferenceOffer[] {
  const response = record(value, "inference offers");
  if (!Array.isArray(response.offers)) {
    throw new Error("Invalid inference offers");
  }
  return response.offers.map((item) => {
    const offer = record(item, "inference offer");
    const disclosure = offer.disclosure;
    if (
      disclosure !== null &&
      (typeof disclosure !== "string" || disclosure.length === 0)
    ) {
      throw new Error("Invalid inference offer field: disclosure");
    }
    return {
      offer_id: text(offer, "offer_id", "inference offer"),
      revision: text(offer, "revision", "inference offer"),
      provider_id: text(offer, "provider_id", "inference offer"),
      disclosure_version: text(offer, "disclosure_version", "inference offer"),
      config_digest: text(offer, "config_digest", "inference offer"),
      disclosure: disclosure as string | null,
    };
  });
}

/** Only the offers the core has words for can be chosen. */
export function selectableOffers(offers: InferenceOffer[]): InferenceOffer[] {
  return offers.filter((offer) => offer.disclosure !== null);
}

function parseSelection(value: unknown): ConnectionSelection | null {
  if (value === null) return null;
  const selection = record(value, "inference selection");
  const version = selection.state_version;
  if (typeof version !== "number" || !Number.isInteger(version)) {
    throw new Error("Invalid inference selection field: state_version");
  }
  return {
    connection_id: text(selection, "connection_id", "inference selection"),
    state_version: version,
    offer_id: text(selection, "offer_id", "inference selection"),
    revision: text(selection, "revision", "inference selection"),
    config_digest: text(selection, "config_digest", "inference selection"),
    disclosure_version: text(
      selection,
      "disclosure_version",
      "inference selection",
    ),
    reselection_required: flag(
      selection,
      "reselection_required",
      "inference selection",
    ),
  };
}

export function parseCurrentConnection(value: unknown): CurrentConnection {
  const response = record(value, "inference connection");
  if (!("selection" in response)) {
    throw new Error("Invalid inference connection field: selection");
  }
  return {
    selection: parseSelection(response.selection),
    installed_on_this_device: flag(
      response,
      "installed_on_this_device",
      "inference connection",
    ),
    pending_install: flag(response, "pending_install", "inference connection"),
    revocation_applied: flag(
      response,
      "revocation_applied",
      "inference connection",
    ),
  };
}

export function parseSelectResult(value: unknown): SelectResult {
  const response = record(value, "inference select");
  if (response.selected !== true) {
    throw new Error("Invalid inference select");
  }
  return {
    connection_id: text(response, "connection_id", "inference select"),
    config_digest: text(response, "config_digest", "inference select"),
    previous_witness_removed: flag(
      response,
      "previous_witness_removed",
      "inference select",
    ),
  };
}

/**
 * Every connection method presents the account session; without one the
 * daemon answers `account-session-required` before any network call. That
 * routes the contributor to sign-in rather than reading as a failure.
 */
export function needsAccountSignIn(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message : typeof error === "string" ? error : "";
  return message.includes("account-session-required");
}

/**
 * What install acts on: a selection this device holds and the account
 * still reports, never one made on another device, one already installed
 * here, or a retired revision.
 */
export function installTarget(
  current: CurrentConnection,
  justSelected: SelectResult | null,
): InstallTarget | null {
  const selection = current.selection;
  if (current.installed_on_this_device) return null;
  if (selection?.reselection_required) return null;
  if (justSelected) {
    return {
      connection_id: justSelected.connection_id,
      config_digest: justSelected.config_digest,
    };
  }
  if (selection && current.pending_install) {
    return {
      connection_id: selection.connection_id,
      config_digest: selection.config_digest,
    };
  }
  return null;
}

export type InferenceView =
  | { kind: "loading" }
  | { kind: "sign_in" }
  | { kind: "none" }
  | { kind: "installed"; connectionId: string }
  | { kind: "install"; target: InstallTarget }
  | {
      kind: "choose";
      offers: InferenceOffer[];
      /** The account's selection is not on this device: another one has it. */
      otherDevice: boolean;
      /** The selection's revision was retired; the contributor chooses again. */
      reselect: boolean;
      /** `state_version` to pass when replacing the account's selection. */
      expectedVersion: number | null;
    };

export function inferenceView(input: {
  signedIn: boolean | null;
  offers: InferenceOffer[] | null;
  current: CurrentConnection | null;
  justSelected: SelectResult | null;
  loading: boolean;
}): InferenceView {
  if (input.signedIn === false) return { kind: "sign_in" };
  if (
    input.loading ||
    input.signedIn === null ||
    input.offers === null ||
    input.current === null
  ) {
    return { kind: "loading" };
  }
  const current = input.current;
  const selection = current.selection;
  if (
    selection &&
    current.installed_on_this_device &&
    !selection.reselection_required
  ) {
    return { kind: "installed", connectionId: selection.connection_id };
  }
  const target = installTarget(current, input.justSelected);
  if (target) return { kind: "install", target };
  const offers = selectableOffers(input.offers);
  if (offers.length === 0) return { kind: "none" };
  const reselect = selection?.reselection_required === true;
  return {
    kind: "choose",
    offers,
    otherDevice:
      selection !== null && !reselect && !current.installed_on_this_device,
    reselect,
    expectedVersion: selection?.state_version ?? null,
  };
}
