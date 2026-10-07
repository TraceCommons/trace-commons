// Kept free of imports so the node test runner can load it directly.

/**
 * The shell's one sentence for when the core's words did not arrive. It
 * cannot come from the core, because the core's words are what failed, so
 * it is the only status sentence this shell types. It says what is true in
 * every case -- the wording could not be read -- and claims nothing about
 * what is running, waiting or sent. DRAFT, NEEDS APPROVAL (new, 2026-10-06).
 */
export const WORDING_UNREADABLE =
  "This can't be shown, because this build could not read its wording.";

/**
 * The quit prompt's heading and buttons when the core's quit prompt could
 * not be read. A contributor must never be trapped in the app, so the
 * buttons need words; these are the core's own (`quit_copy`), copied here
 * only because the core's answer is what failed to arrive. The body is
 * `WORDING_UNREADABLE`, never a sentence about what keeps running.
 */
export const QUIT_FALLBACK = {
  title: "Quit Trace Commons?",
  confirm: "Quit",
  cancel: "Cancel",
} as const;
