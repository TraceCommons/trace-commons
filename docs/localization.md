# Shared translation catalog

The **Rust copy tables remain canonical English**. `localization/catalog.json`
mirrors their fixed prose for translation; it is not a new authority for shared
English. Change English in its owning Rust table, rebuild the contributor FFI,
and import. Do not fix an English mismatch by editing only the catalog.

The macOS `TCLocalization` target consumes generated native resources. Thirty
fixed-copy bridge calls cover first run, settings, menu-bar text, tooltips,
accessibility copy, privacy disclosures, confirmations and fixed errors.
Dynamic presentations also cross the ABI as explicit keys, canonical source
templates and named parameters. Rust still selects privacy claims, suppression,
refusals and handle-derived quit roles. Existing English APIs stay compatible.
Only English is enabled initially; no
translated locale has been approved or shipped. There are no Python dependencies
(Python 3.9+).

## Language-picker rollout

Settings uses the daemon's `localization_language_selection` feature decision.
The flag is default-off and live: missing/unavailable snapshots or a remote
disable hide the picker without clearing `AppleLanguages`. Saved preferences,
system-language resolution, native resources and English fallback keep working.
Staging and testing profiles permit remote enablement; production's allowlist
does not yet permit it. Enabling production requires a reviewed profile change
and compatible release, not just a remote manifest. No manifest is published by
this code change. Translation availability and human-review requirements remain
independent of this flag; it cannot enable an unapproved locale.

## Generate and verify

From the repository root, using a **freshly built, trusted** FFI library:

```sh
# Build only if another task is not already building this library.
CARGO_INCREMENTAL=0 RUSTFLAGS='-D warnings' cargo build -p trace-commons-contributor-ffi

# Linux orb example; use target/debug/libtrace_commons_contributor_ffi.dylib on macOS.
python3 scripts/localization/catalog.py \
  --library target/debug/libtrace_commons_contributor_ffi.so \
  --import --snapshot localization/generated/ffi-snapshot.json

python3 -m unittest discover -s scripts/localization -v
python3 scripts/localization/catalog.py \
  --library target/debug/libtrace_commons_contributor_ffi.so \
  --check --snapshot localization/generated/ffi-snapshot.json
python3 scripts/localization/catalog.py --report
```

`--library` loads native code with `ctypes`, calls only the explicit registry of
no-argument table exports and `tc_message_templates_json`, parses owned JSON strings, and frees every non-null
pointer with `tc_string_free`, including on parse failure. Do not load untrusted
libraries. A stale library can only prove agreement with that stale build;
rebuild after changing Rust. The tests also pin registry coverage against the
fixed copy export declarations in the checkout.

`--import` adds new keys with authored surface context plus their full field
path, preserves existing contexts/review classifications/translations, replaces
mirrored English and parameters, and removes retired fields in registered table
namespaces. It never updates translation hashes or marks a translation reviewed.
An old translation hash intentionally becomes stale. If an enabled locale is
affected, set it disabled before import and review it again before re-enabling;
otherwise import fails closed without writing any outputs.
Stale disabled translations retain their old parameter contract for review;
their placeholder syntax still must be valid. A translation claiming a current
hash must match the new parameter contract and protected tokens exactly.

`--check` requires the real library. It checks source equality, validation and
byte-for-byte output equality without writing. Include `--snapshot` as above to
also verify the checked-in export/exclusion report. Extra generated locale
`.strings`, `.stringsdict` or `InfoPlist.strings` files are rejected, including
bundles for a locale that has since been disabled. A normal generation removes those obsolete
generator-owned files; it does not remove unrelated resource files.
The client CI runs this check after rebuilding the FFI library, runs the Python
and Swift tests, and uploads `macos-localization-review` screenshots of English,
expanded and RTL settings, onboarding and menu-bar states. Enabling another
language also deliberately changes the production Info.plist golden fixtures
under `scripts/ci/tests/fixtures/`.

Without flags, the script validates and regenerates from the catalog, useful
while translating offline; that is **not** live FFI verification. `--resources`
and `--generated` can redirect outputs for isolated tests. `--report` writes
nothing and prints per-non-English-locale missing/draft/outdated totals, even
when a mistakenly enabled locale is incomplete. Draft and outdated can overlap.
An empty report means no non-English locales are registered, not that any
translation has passed review.

Generated files:

| Path | Purpose |
| --- | --- |
| `macos/Sources/TCLocalization/Resources/Catalog.json` | Same version/sourceLanguage/locales/messages schema as source, with only enabled locales and their translations; includes known original English, context, parameters and review classification |
| `macos/Sources/TCLocalization/Resources/{locale}.lproj/Localizable.strings` | Fixed native strings; always includes English |
| `macos/Sources/TCLocalization/Resources/{locale}.lproj/Localizable.stringsdict` | Native plural rules; always includes English |
| `macos/Sources/TCLocalization/Resources/{locale}.lproj/InfoPlist.strings` | `NSAppleEventsUsageDescription` from `app.permission.apple_events`; app assembly also copies this into the **main** app's `Contents/Resources/{locale}.lproj/` for OS permission prompts |
| `localization/generated/Localizable.xcstrings` | Translator exchange/view, including disabled locales' drafts; **outside** SwiftPM Resources to avoid duplicate compilation |
| `localization/generated/locales.plist` | XML array of enabled locale IDs for app packaging / `CFBundleLocalizations` |
| `localization/generated/ffi-snapshot.json` | Auditable mirrored prose, explicit registry, and field exclusion reasons; checked when `--snapshot` is supplied |

`TCLocalization` uses `.process("Resources")`: `Catalog.json` is at the
`Bundle.module` root, and each native table is in its language bundle. Packaging
should read `locales.plist`, not maintain a second language list. The catalog and
English bundle guarantee a known fallback even when the selected bundle lacks a
resource; the runtime must explicitly fall back to canonical English on failed
lookup or source mismatch.

## Translation and review

Add a locale such as `"fr": {"name": "Français", "enabled": false}`. Supported
locale ID syntax is lowercase two/three-letter language, optional title-case
four-letter script, optional uppercase two-letter or three-digit region:
`fr`, `pt-BR`, `zh-Hans`, `zh-Hant-TW`, `es-419`. Underscores, arbitrary variants,
path components and production pseudo IDs are rejected.

Each message has `english` (string, or plural object), nonempty `context`,
`parameters` (unique named placeholders), `review` (`standard` or `sensitive`),
and `translations`. A translation entry is:

```json
{"value": "Bonjour {name}", "state": "draft", "sourceHash": "<64 lowercase hex characters>"}
```

Use the exact current English source hash. For a string, this is SHA-256 of its
UTF-8 bytes, with **no normalization or trimming**. For a plural object, hash
UTF-8 compact JSON with sorted keys, `ensure_ascii=False`, separators `(',', ':')`.
The helper `source_hash` in `scripts/localization/catalog.py` implements this.
Changing the hash is not a substitute for reviewing changed source text.

Use `Localizable.xcstrings` for translator context and exchange, but transfer
translations back into `catalog.json`; generated xcstrings are overwritten.
There is intentionally no automatic translator-file import or automatic review
promotion. For plural strings the exchange view uses `%lld`; convert that count
back to `{count}` in the catalog, and decode escaped literal `%%` back to `%`.

Named placeholders can be reordered, not renamed, added or removed. Examples:
`{tool}`, `{name}`, `{date}`, `{count}`. Positional fields, formatter directives
inside braces, unmatched braces, and control characters are invalid. Preserve
commands, paths, URLs, protected backtick snippets and product names verbatim;
the validator checks the protected-token patterns declared in the generator.
New technical tokens need an explicit protection rule or field exclusion before
translation. Follow [the UI copy guide](ui-copy-guide.md): short actions, distinct
titles/accessibility labels, and consequences beside the decision.

Every message in an enabled non-English locale must have `state: "reviewed"`
and a `sourceHash` matching its current English. **No partial locale or draft
translation ships**, including disabled translations in runtime metadata. Keep
`enabled: false` until all messages are reviewed. A translation cannot erase a
visible disclosure or restore a deliberately suppressed notice. A native speaker must review
all text; `sensitive` text additionally needs deliberate review of consent,
disclosure, privacy, withdrawal, account migration and destructive consequences
against the original Rust semantics. Do not strengthen uncertain privacy claims,
omit limitations, or turn a local observation into a verified outcome. Record
reviewer/locale evidence in the change review; `reviewed` is a human assertion,
not a machine translation quality score. Rust's own draft/owner-approval status
is independent of translation review; this import does not approve Rust copy.

## Native plurals and safe runtime contract

Plural English requires exactly `one` and `other`, with numeric `count` in
parameter metadata. Translations require `other`, and may use only `zero`,
`one`, `two`, `few`, `many`, `other`. A form may omit `{count}` when its words
already express the quantity (for example, “once”); otherwise it appears at
most once. All other named parameters must remain present. Apple selects the locale's plural
category; do not implement `count == 1` in the client for translated strings.

The `.stringsdict` uses `NSStringPluralRuleType`, format `%#@count@` and
`NSStringFormatValueTypeKey = lld` (signed long long). Each count is `%lld`;
literal `%` is escaped as `%%` before inserting that directive. Other named
parameters remain `{name}`. Nonplural `.strings` retain literal `%` unchanged
and must not be passed through printf formatting.

The native runtime:

1. Localize only explicit fixed-table bridge calls with their namespace.
   Namespace is the exact C symbol without `tc_` and trailing `_json`, retaining
   `_copy`: `tc_first_run_copy_json` becomes `first_run_copy`.
2. Recursively derive `namespace.json.field.path` (audited static array indices
   included). Only replace a string if that exact key exists and its original
   incoming English equals metadata `english`. Unknown fields, excluded
   identifiers and mismatched English stay untouched. No global text search.
3. For an explicit plural key, call native
   `String(format: template, locale: selectedLocale, arguments: [count])`
   over the bundle's localized native format with a signed 64-bit count.
   Pass the selected locale explicitly, including in DEBUG pseudo previews.
   Use the English bundle's plural rule when the selected locale/key is
   unavailable; a raw `{count}` fallback is not a native format template.
4. Substitute remaining named values in **one pass after** lookup/formatting.
   Never parse a substituted value again: a user value containing `{other}` or
   `%lld` remains literal. Never pseudo-transform or localize trace bodies,
   titles, prompts, file contents, folder names, account identifiers, commands,
   error labels, API facts, or URLs supplied by a user/service.

Seven explicit non-table seeds support the language title/system choice/restart
text, first-run window title, Apple Events permission prompt, and residual-secret
warning. The permission seed is sensitive and reads exactly
"Near.ai opens Private AI sessions in Terminal." The residual
unchecked sentence is copied exactly from `preview_copy.rs`; the count seed
preserves its one/other English wording. These are explicit runtime calls, not
permission to localize other dynamic FFI strings by matching their text.

`localization/macos-messages.json` holds existing shell-owned text with authored
translator context and review classification. It cannot shadow a Rust-owned
key. `localization/plurals.json` maps native plural keys to canonical one/other
table paths and their original count parameter; import derives their English
from the live library rather than duplicating it in the manifest.

Dynamic calls use `tc_localized_presentation_json(operation, input_json)`.
The operation is explicitly dispatched; input consists of facts, never completed
English to search for. Prose leaves carry `{key, english, parameters}`. Other
fields remain ordinary JSON. `TCLocalization.presentationJSON` checks source
agreement before lookup, recursively renders nested messages, applies native
number/currency formatting to typed values, and performs single-pass named
substitution. Unknown keys or changed source templates fall back to the incoming
canonical English. The generic endpoint rejects `quit_prompt`; only
`tc_localized_quit_prompt_json(handle)` can select the actual watcher role.
Unknown operations and invalid input return NULL; nudge helpers preserve their
legacy empty-object behavior on invalid input.

## Audited export scope

The explicit registry currently contains these 32 namespaces:

```text
routing_copy consent_copy attach_copy private_inference_copy public_run_copy
skill_learning_copy route_disclosure_unreadable_copy certificate_detail_copy
session_notification_copy source_settings_copy witness_copy onboarding_copy
first_run_copy contribution_mode_copy missions_disclosure_copy
monitor_traces_copy monitor_screens_copy shell_words_copy
inference_connection_copy payout_link_copy referral_copy account_devices_copy
account_change_copy set_up_again_copy sent_traces_training_copy privacy_scan_copy
contributor_disclosure_copy managed_copy insights_copy nudge_copy compute_copy
legacy_migration_offer
```

The first 30 cover the fixed `tc_*copy*` exports in FFI `lib.rs`, `insights.rs`
and `nudge.rs`; `compute.rs` adds `compute_copy`, and the non-copy-named fixed
table adds `legacy_migration_offer`. Registry membership alone does not mean a
client consumes every key: missions disclosure may still need an explicit
runtime consumer. Notification prose uses the keyed presentations described
below rather than translating completed notification strings. `contributor_disclosure_copy`
duplicates some standalone copy under its own paths, intentionally, because
those are distinct explicit FFI entry points.

Audited static lists: first-run Rules weekdays/months; contribution-mode choices;
public-run outcome/feedback/evidence/status/history/credit/use/reuse choices;
public-profile published/never lines. All other lists fail closed until audited.

Excluded fields (full concrete paths/reasons are in the snapshot and printed on
every live export):

- `account_devices_copy.code_command`: executable login command.
- `routing_copy.folder_note`: machine-specific path, now supplied as a named
  parameter by the `routing_folder_note` presentation rather than catalogued
  from whichever home directory ran the generator.
- `first_run_copy.join.invite_placeholder`: invitation URL example.
- `referral_copy.field_placeholder`: referral-code format, not prose.
- `witness_copy.url_placeholder`: example URL.
- `witness_copy.signing_address_placeholder` and `.measurements_placeholder`:
  address/measurement identifier examples, not prose.
- `shell_words_copy.withdrawal.not_found_labels`: wire error identifiers.
- Public-run choice `.value` and reuse `.permission`: wire identifiers, not
  labels. The neighboring label/explanation fields remain translatable.
- Contribution-mode choices' `.mode`: wire identifier.
- `first_run_copy.uses.credit_scope`: protocol permission scope.
- `skill_learning_copy.contract_summary_format`, `.model_budget_format` and
  `.review_budget_format`: legacy printf contracts. Native skill budgets use
  named, typed presentations instead; translating these positional formats
  would change their legacy formatting contract.
- Source tool `.key` fields in `source_settings_copy` and
  `contributor_disclosure_copy.source_settings`: source identifiers used by permissions.
- Non-string metadata (including disclosure state `working` booleans).
- `tc_install_guides_json` is omitted entirely; install commands, URLs, paths
  and identifiers are not safely prose. No arbitrary `tc_*` APIs are called.

## Development pseudo and screenshot review

```sh
python3 scripts/localization/catalog.py --pseudo /tmp/trace-localization-pseudo

# macOS only: launch an already-built DEBUG app directly so it inherits env.
TC_LOCALIZATION_RESOURCES=/tmp/trace-localization-pseudo \
  macos/.build/Near.ai.app/Contents/MacOS/TraceCommonsApp -AppleLanguages '(en-XA)'

# Quit fully between runs. The direction options also exercise mirrored layout.
TC_LOCALIZATION_RESOURCES=/tmp/trace-localization-pseudo \
  macos/.build/Near.ai.app/Contents/MacOS/TraceCommonsApp -AppleLanguages '(ar-XB)' \
  -AppleTextDirection YES -NSForceRightToLeftWritingDirection YES
```

This writes only development bundles to a separate directory: `en-XA` expands
and accents fixed prose; `ar-XB` wraps it in RTL isolates. Placeholders, protected
tokens and intentionally empty/suppressed copy are preserved. It writes no
production metadata, locale list, or resources, and is not an approved Arabic
or other translation. The separate output is a usable debug bundle directory:
`Catalog.json` with `en`, `en-XA`, `ar-XB` enabled, English fallback plus both
pseudo `.lproj` native tables and `InfoPlist.strings`. Its synthetic translation
entries carry current source hashes and the schema's `reviewed` state solely
for development selection, **not human approval**; every context is marked
DEVELOPMENT SYNTHETIC — NOT HUMAN REVIEWED. The production validator rejects
these locale IDs. The DEBUG-only `TC_LOCALIZATION_RESOURCES` loader selects this
directory; release builds ignore the override. Never copy it into a release or
the production language picker. Previewing OS permission prompts also requires
a disposable debug app's main-bundle localized `InfoPlist.strings`; overriding
the app's translation resource directory alone cannot override Apple's prompt.

The concrete native screenshot test is
`NativeOnboardingRenderTests.testLocalizedSettingsOnboardingAndMenuRender`.
After generating the pseudo bundle, run on a macOS host/runner (fresh process
per language; `TC_LOCALIZATION_LANGUAGE` is also DEBUG-only):

```sh
for language in en en-XA ar-XB; do
  TC_LOCALIZATION_RESOURCES=/tmp/trace-localization-pseudo \
  TC_LOCALIZATION_LANGUAGE="$language" \
  TRACE_COMMONS_SCREENSHOT_DIR="$PWD/.amp/in/artifacts/localization/$language" \
    swift test --package-path macos \
      --filter NativeOnboardingRenderTests
done
```

It renders `language-settings-narrow.png` (360px),
`language-onboarding.png` (860px), `language-menu.png` (360px), and the existing
witness consent, contribution, account and data-use states per language.
Review the images rather than treating render success as visual approval.
This covers English and synthetic expansion/RTL, not real CJK, translated
consent correctness, OS permission prompts, or all states in the matrix below.

On macOS the language setting uses application `AppleLanguages` preferences and
requires a full app restart; selecting **System default** removes the app
override rather than setting a synthetic language ID. Persist only enabled
locale IDs. Test a normal restart, restoring system preference, and fallback
when a previous preference is no longer enabled. Do not promise live language
switching. For Apple's own UI pseudo testing, launch a development build with
`-NSDoubleLocalizedStrings YES`, `-NSShowNonLocalizedStrings YES`, and separately
`-AppleTextDirection YES -NSForceRightToLeftWritingDirection YES`; these test
expansion/unlocalized markers/layout independently of the generated test bundles.
Use real reviewed CJK/RTL locales with `-AppleLanguages '(ja)'` / `'(ar)'` only
when available; pseudo output cannot prove native-speaker correctness.

Before enabling a locale, capture and review this matrix on a real macOS host:

| Surface | Required cases |
| --- | --- |
| Narrow menu and menu-bar popover | Long/expanded actions, selected state, accessibility names, no clipped shortcuts |
| Settings and language picker | Minimum width, restart hint, system default, fallback, keyboard/VoiceOver order |
| Onboarding / first-run | Narrow window, multi-line consent, disabled/error states, date labels |
| Privacy / withdrawal / destructive confirmations | Full consequences visible, count 0/1/2/large, unavailable scan distinct from zero |
| RTL | Mirrored layout and traversal, mixed URLs/commands/numbers, isolated user values unchanged |
| CJK | Wrapping, line breaks, font fallback, no forced English-width controls |

These screenshots and Apple plural/resource compilation cannot be verified in a
Linux orb. Python tests validate generation and the boundary model, not SwiftUI
layout or Apple's actual locale engine.

### macOS migration boundaries

The keyed presentation layer covers routing discovery/port/token/folder/time,
source state, witness results/counts/review failures, Private AI state,
eligibility/attestation, spending/balance, certificates, enrollment/refusal
messages, project ignore/arming/reconciliation, automatic-contribution consent,
grant and route disclosures, migration/account notices, health and queue limits,
withdrawal and handle-derived quit confirmation. Redaction summaries preserve
survivor identifiers while localizing known family prose. Compute snapshots
preserve lifecycle facts and localize explicit source-owned prose.

Notification cards, menu-bar marks, standalone notifications and digest folds
carry additive keyed presentations alongside their existing English fields.
Rust selects suppression, zero-clause omission, credit precision and action IDs
from the same facts as the English rendering. macOS renders the presentation
recursively, including nested tool/count/credit/date messages; tool and project
names remain literal parameters. Translated action labels are used only when
their IDs match the original action sequence exactly. Older payloads without
presentations retain their original English, and intentionally empty notification
settings copy stays empty.

The three `skill_learning_*_line` budget operations (contract summary, model
budget and review budget) use canonical named templates shared with the legacy
printf exports. Integer parameters retain signed 64-bit precision and request
native number formatting; owner names remain literal parameters. Missing or
wrongly typed inputs are rejected rather than inferred from formatted text.

Shell consumers now use native plural keys instead of English one/other
selection for the mapped table fields, and use the selected presentation locale
for UI dates, numbers, percentages and currency. Canonical rounding, precision,
unavailable states and subcent decisions remain core-owned.

Other clients can reuse the common catalog and presentation ABI, but their
runtime migrations are not part of the macOS-first implementation. Keep
OS-supplied labels, executable install guidance, protocol identifiers, user and
service content outside translation. Unknown future detector categories remain
technical labels rather than guessed prose. The existing ByteCountFormatter
retains system-locale behavior. Real CJK/RTL review, Apple plural execution,
permission prompts, keyboard/VoiceOver behavior and native screenshots remain
macOS verification gates, not claims established by Linux checks.
