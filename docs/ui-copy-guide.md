# UI copy rules

Use these rules when adding or changing Trace Commons product text. They are
based on [PR #1288](https://github.com/TraceCommons/trace-commons/pull/1288),
“Cut button labels to a verb or a verb and an object,” and its October 8, 2026
button rule. That PR was open when this guide was written; this guide records
its editorial decisions, not a claim that the changes have shipped.

## 1. Buttons name actions

Use an action verb, or a verb and an object. Aim for one or two words; longer
labels are exceptions when a shorter label would be ambiguous or misleading.
Do not impose the same word limit on explanations or confirmations.

| Before | Preferred |
| --- | --- |
| Keep on this Mac | Keep |
| Request server refresh | Refresh |
| Custom setup instead | Customize |
| Send this session for review | Send |
| Move to my NEAR AI account | Move |
| Create comparison task | Create task |
| Save user-reported outcome | Save outcome |

Use the nearby section, selected item, or dialog to establish the object.
Keep the object in the button when that context is insufficient. Do not replace
a precise action with a vague “OK” simply to shorten it.

## 2. Name what this click does

A button that opens settings is **Review settings**, even if the user can later
turn a feature on there. Reserve **Turn on** for the action that enables it.
A notice's **Dismiss** must not use the session-decline wording **Not this one**.
These are different actions and need separate copy keys.

Prefer the user's action over machinery: **Refresh**, not **Request server
refresh**. Keep implementation details in explanatory text only when they help
the user make a decision.

## 3. Move essential context beside the action

Shorten the button without deleting information needed before clicking it.

| Button | Context that stays visible |
| --- | --- |
| Load more | `{size} remaining` |
| Save | A saved specification cannot be changed. |
| Skip | What skipping sets up, and what still requires sign-in |
| Send | The destination and scope of the submission |

Consequences, immutability, scope, consent, and destructive effects belong in
nearby copy or the confirmation. A tooltip or accessibility name alone is not
an adequate replacement for information every user needs to decide.

## 4. Give titles and buttons separate jobs

A title identifies the task; a button performs the action. Do not reuse one
string for both just because they initially have the same wording.

| Title | Button |
| --- | --- |
| Create new passkey | Create |
| Launch managed session | Launch |
| Clear episode assessment | Clear |

Use distinct keys such as `launch_title`, a button key, and an accessibility
key where needed. Preserve the full confirmation title when shortening its
trigger. Separate actions may have identical visible labels and still need
independent keys.

## 5. Keep repeated controls distinguishable to assistive technology

Several controls may visibly say **Save**, **Refresh**, or **Delete** when their
sections make the target clear. Give them distinct accessible names, such as
**Save episode assessment** and **Refresh saved specifications**. Include the
visible action in the accessible name.

Wire these names into each affected shell's accessibility API; adding a copy
key without consuming it is insufficient.

## 6. Navigation names things; choices name values

Use short noun phrases for sections: **Data uses**, **Change log**. Keep the
section heading and navigation label consistent.

A duration choice is a value rather than a command: **1 hour**, **Until
morning**, **Until resumed**. Do not force the verb rule onto every menu item,
field label, status, or heading.

## 7. Update references when a label changes

Help and recovery text must name the button the user can actually see:
**Choose Verify account** and **Choose Refresh**. Search for the old wording in
hints, confirmations, accessibility names, fixtures, tests, and documentation.
Do not blindly replace it everywhere: the old full wording may remain correct
as a dialog title or accessible name.

## 8. Keep shared copy shared

Edit the owning Rust copy table or service and update its consumers in macOS,
GTK, and Windows. Avoid introducing shell-local prose to work around a missing
shared field. When adding a field across the daemon/FFI boundary, preserve
compatibility with older payloads where required.

PR #1288 explicitly left Tauri's own copies unchanged. Preserve that scope for
this rule set; do not treat it as permission for a blanket Tauri rewrite or as
an exemption from checking consumers of changed shared fields.

## Review checklist

- Does each action label describe the immediate action, usually in 1–2 words?
- Is the target clear from the label and surrounding context?
- Are consequences and scope still visible before the decision?
- Do titles, buttons, and accessible names have the appropriate separate keys?
- Can assistive technology distinguish repeated controls?
- Do headings and hints match the current navigation and controls?
- Have all affected shared-copy consumers and their tests been updated?
- Have copy contracts, bridge/decoder tests, wording ratchets, and relevant UI
  checks been run? Report platforms that were not compiled or exercised.

Update wording baselines only to reflect actual removals or migration of
shell-local copy; do not raise them to hide new prose. PR #1288's dated approval
comments record an actual editorial decision. Do not copy those comments onto
new text and imply approval that has not occurred.
