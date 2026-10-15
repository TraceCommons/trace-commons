// Kept free of imports so the node test runner can load it directly.

/**
 * The status lines a screen shows around a failed read or request, from
 * `health_copy::shell_status_copy`: the core-down banner, the line for a
 * read that did not arrive, the line for a request that did not go through,
 * and the start-again button's two states.
 */
export type ShellStatusCopy = {
  core_down: { title: string; detail: string };
  read_unavailable: string;
  request_failed: string;
  retry_startup: string;
  retrying_startup: string;
};

type RecordValue = Record<string, unknown>;

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid ${label}`);
  }
  return value as RecordValue;
}

function text(value: RecordValue, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid status copy field: ${key}`);
  }
  return field;
}

/** Refuses, rather than fills in, a payload missing any line. */
export function parseShellStatusCopy(value: unknown): ShellStatusCopy {
  const item = record(value, "status copy");
  const coreDown = record(item.core_down, "core-down copy");
  return {
    core_down: {
      title: text(coreDown, "title"),
      detail: text(coreDown, "detail"),
    },
    read_unavailable: text(item, "read_unavailable"),
    request_failed: text(item, "request_failed"),
    retry_startup: text(item, "retry_startup"),
    retrying_startup: text(item, "retrying_startup"),
  };
}

export type ShellStatusLines = {
  /** The core's words could not be read; every line is the shell's one. */
  unreadable: boolean;
  coreDown: { title: string; detail: string };
  readUnavailable: string;
  requestFailed: string;
  retryStartup: string;
  retrying: string;
};

/**
 * The lines a screen draws: the core's words once they arrive, the shell's
 * one sentence (`unreadableSentence`, `WORDING_UNREADABLE`) if they cannot
 * be read, and nothing while they load. A button has no fallback words, so
 * a screen that cannot read them offers none.
 */
export function shellStatusLines(
  copy: ShellStatusCopy | undefined,
  failed: boolean,
  unreadableSentence: string,
): ShellStatusLines {
  if (copy) {
    return {
      unreadable: false,
      coreDown: copy.core_down,
      readUnavailable: copy.read_unavailable,
      requestFailed: copy.request_failed,
      retryStartup: copy.retry_startup,
      retrying: copy.retrying_startup,
    };
  }
  const line = failed ? unreadableSentence : "";
  return {
    unreadable: failed,
    coreDown: { title: line, detail: "" },
    readUnavailable: line,
    requestFailed: line,
    retryStartup: "",
    retrying: "",
  };
}
