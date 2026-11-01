// Trace Commons economizer for a managed OpenCode session, written by the
// launcher into the session's own profile for each launch, with SETTINGS
// filled in: the hook program with its arguments, and the managed
// session's id. It translates OpenCode's plugin events into the hook
// events `economizer-hook` reads, runs that program without a shell, one
// event at a time, and gives what it answers to the model. Any failure is
// ignored, so a broken economizer leaves the session as if it were absent.
import { spawn } from "node:child_process";

const SETTINGS = __TRACE_COMMONS_SETTINGS__;
const TIMEOUT_MS = 10000;
const LIMIT = 1024 * 1024;

function run(input) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(SETTINGS.argv[0], SETTINGS.argv.slice(1), {
        stdio: ["pipe", "pipe", "ignore"],
        windowsHide: true,
      });
    } catch {
      resolve(undefined);
      return;
    }
    let out = "";
    const timer = setTimeout(() => child.kill(), TIMEOUT_MS);
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      if (out.length < LIMIT) out += chunk;
    });
    child.on("error", () => {
      clearTimeout(timer);
      resolve(undefined);
    });
    child.on("close", () => {
      clearTimeout(timer);
      try {
        const answer = out.trim() ? JSON.parse(out) : undefined;
        const context = answer?.hookSpecificOutput?.additionalContext;
        resolve(typeof context === "string" ? context : undefined);
      } catch {
        resolve(undefined);
      }
    });
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify({ ...input, session_id: SETTINGS.session }));
  });
}

// Events reach the program in the order they happened: a session's end
// must not be folded before its last turn is.
let queue = Promise.resolve();
function hook(input) {
  const next = queue.then(() => run(input));
  queue = next;
  return next;
}

export const TraceCommonsEconomizer = async () => {
  const facts = await hook({ hook_event_name: "SessionStart" });
  const ours = (id) => id === SETTINGS.session;
  let ended = false;
  const end = async () => {
    if (ended) return;
    ended = true;
    await hook({ hook_event_name: "SessionEnd" });
  };
  return {
    "experimental.chat.system.transform": async (input, output) => {
      if (facts && ours(input.sessionID)) output.system.push(facts);
    },
    "chat.message": async (input) => {
      if (ours(input.sessionID)) await hook({ hook_event_name: "UserPromptSubmit" });
    },
    "tool.execute.after": async (input, output) => {
      if (!ours(input.sessionID)) return;
      const exit = output?.metadata?.exit;
      const failed = typeof exit === "number" && exit !== 0;
      const text = typeof output?.output === "string" ? output.output : "";
      const context = await hook({
        hook_event_name: failed ? "PostToolUseFailure" : "PostToolUse",
        tool_name: input.tool,
        tool_input: input.args ?? {},
        // The exit status rides beside the text, so a passing check that
        // logs `error:` is not read as failed.
        ...(failed
          ? { error: text }
          : { tool_response: typeof exit === "number" ? { output: text, exit_code: exit } : text }),
      });
      if (context && typeof output?.output === "string") output.output += "\n\n" + context;
    },
    event: async ({ event }) => {
      if (event?.type === "session.idle" && ours(event.properties?.sessionID)) {
        await hook({ hook_event_name: "Stop" });
      }
    },
    dispose: end,
  };
};
