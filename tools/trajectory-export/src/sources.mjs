// The transcript sources upstream normalizes, taken from
// @letta-ai/trajectory 0.3.0's TrajectorySource union.
//
// claude-code and codex are deliberately absent. The Rust CLI reads both
// natively, straight out of their local stores, with no conversion step and no
// Node on the machine. Routing them through here would be strictly worse and
// would also perturb the event extraction the pilot has already scored
// against.
//
// deepagents is absent for a different reason: it is a checkpoint source, not
// a transcript one, and needs loadDeepAgentsCheckpoint/normalizeCheckpoint
// rather than normalizeTranscript. Supporting it is a separate change, not an
// entry in this list.
export const SOURCES = [
  "atif",
  "copilot-cli",
  "cursor",
  "droid",
  "gemini-cli",
  "hermes",
  "letta-code",
  "omp",
  "openclaw",
  "opencode",
  "openhands",
  "pi",
];

export const NATIVE_SOURCES = ["claude-code", "codex"];
