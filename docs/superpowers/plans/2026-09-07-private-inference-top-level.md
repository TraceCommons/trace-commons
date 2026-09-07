# Private Inference Top-Level Destination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote private inference out of Settings into a top-level destination in all three shells, with a tray/menu-bar toggle and in-app keyboard shortcuts.

**Architecture:** Almost everything needed already exists — `PrivateInferenceSurface` computes the state line, serving line and tone; `private_inference_copy.rs` owns every word; all three shells already have a tray surface. This plan adds two nav copy fields in Rust, then wires the existing surface into a new destination and into the three trays. No new subsystem.

**Tech Stack:** Rust (`trace-commons-contributor`, `-ffi`, `-contributor-gtk`), Swift/SwiftUI (macOS), C#/WinUI (Windows), GTK4/libadwaita (Linux).

**Spec:** `docs/superpowers/specs/2026-09-07-private-inference-top-level-design.md`

## Global Constraints

- **The user-facing label is "Model calls", never "Private inference".** A sweep
  (`the_offer_surface_says_nothing_it_should_not`) reads this module's own source
  between the `PRIVATE-INFERENCE-SURFACE-BEGIN`/`-END` markers and fails on the
  words `private`, `secure`, `proxy`, `backend`, `route`, `localhost` and vendor
  names. The module header explains why: turning this on does **not** make calls
  private -- it moves where they are answered and keeps the record here, and each
  call still goes on to whoever was configured to answer it. The setting's
  internal name says `private`; the surface must not repeat it as a promise. The
  sweep also asserts the constants have not been moved out from between its
  markers, so relocating them to dodge it is itself a failure. Read `destination`
  from `private_inference_copy::DESTINATION`; never retype the label in a shell.

- **Only `Clear` may be painted as working.** `PrivateInferenceTone` is `Neutral | Held | Clear | Attention | Refused`. `Held`, `Attention`, `Refused` and any unknown ABI value must be visually distinct from `Clear` and must never read as "on".
- **Indicators derive from the tone, never from the settings boolean.** The switch reports what was asked for; the indicator reports what is true.
- **No shell authors user-facing private-inference strings.** Every sentence comes from `private_inference_copy.rs`. If a string is needed that does not exist, add it there first.
- **Do not reuse `RoutingTone.fromABI`** for this surface. `PrivateInferenceSurface.swift:73` documents why: it answers `.neutral` for every value here, turning a refusal into "nothing to say".
- **Do not disturb the offer.** `shouldOffer(answered, on) == !answered && !on` must still hold and the first-run offer must still appear exactly once.
- **Declining still writes the marker ALONE**, never the switch — see `offerParams(accepted:)`.
- **GTK may not depend on a tray.** `ui/mod.rs` states GNOME has no system tray; every capability must be reachable from the window, and a tray is only ever a shortcut into it.
- **GTK switcher icons must stay symbolic**, or the icon silently fails to recolour.
- **Global system-wide hotkeys are out of scope** (cut 2).
- Verify with `RUSTFLAGS="-D warnings" cargo check` / `cargo test`; plain `cargo check` does not apply `-D warnings` and CI does.
- The C ABI header exists in two copies which CI enforces byte-for-byte identical. This plan adds no new ABI function, so the header should not change; if it does, both copies change together.

---

### Task 1: Nav copy fields in Rust and Swift

Adds `destination` and `subtitle` to the private-inference copy payload, mirroring `ComputeCopy`. Every later task consumes these. Rust and Swift must move together because a both-directions field-set test compares the exported field set against the declared one.

**Files:**
- Modify: `crates/trace-commons-contributor/src/private_inference_copy.rs:329` (struct), `:356` (constructor)
- Modify: `macos/Sources/TCShellCore/PrivateInferenceSurface.swift:9` (`PrivateInferenceCopy` + `CodingKeys`)
- Test: `crates/trace-commons-contributor/src/private_inference_copy.rs` (inline `mod tests`), `macos/Tests/`

**Interfaces:**
- Produces: `PrivateInferenceCopy.destination: &'static str` (Rust) / `destination: String` (Swift); `subtitle: &'static str` / `subtitle: String?`. JSON keys `destination` and `subtitle` via `tc_private_inference_copy()`.

- [ ] **Step 1: Write the failing Rust test**

In `private_inference_copy.rs`'s test module:

```rust
#[test]
fn the_copy_carries_nav_wording_for_a_top_level_destination() {
    let copy = private_inference_copy();
    assert!(!copy.destination.is_empty(), "the nav item needs a label");
    assert!(!copy.subtitle.is_empty(), "the destination needs a subtitle");
    // The label sits in a sidebar beside Waiting/History/Computer/Settings.
    assert!(
        copy.destination.chars().count() <= 24,
        "nav label too long for the sidebar: {}",
        copy.destination
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor private_inference_copy`
Expected: FAIL — no field `destination` on `PrivateInferenceCopy`.

- [ ] **Step 3: Add the fields**

In the `PrivateInferenceCopy` struct (around line 329), beside `settings_title`:

```rust
    /// The sidebar/switcher label for the top-level destination.
    pub destination: &'static str,
    /// The one line under the title saying what the destination is for.
    pub subtitle: &'static str,
```

Add the constants near the other copy constants:

```rust
pub const DESTINATION: &str = "Model calls";
const SUBTITLE: &str = "Answer model calls on this computer, and who may use it.";
```

And in `private_inference_copy()` (around line 356):

```rust
        destination: DESTINATION,
        subtitle: SUBTITLE,
```

- [ ] **Step 4: Run the Rust test**

Run: `cargo test -p trace-commons-contributor private_inference_copy`
Expected: PASS.

- [ ] **Step 5: Add the fields on the Swift side**

In `PrivateInferenceCopy`, beside `settingsTitle`:

```swift
    public let destination: String
    public let subtitle: String
```

And in `CodingKeys`:

```swift
        case destination
        case subtitle
```

- [ ] **Step 6: Verify both directions**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor-ffi`
Run: `cd macos && swift test`
Expected: PASS. The `CaseIterable` field-set test compares Rust's exported keys against Swift's declared ones in both directions; if it fails, one side is missing a field.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/private_inference_copy.rs \
        macos/Sources/TCShellCore/PrivateInferenceSurface.swift
git commit -m "Give private inference nav wording for a top-level destination"
```

---

### Task 2: The tone rule, as a shared test

Pins the constraint the whole change hangs on, before any shell renders an indicator. This task adds no UI.

**Files:**
- Test: `macos/Tests/TCShellCoreTests/` (new `PrivateInferenceToneTests.swift`)
- Test: `crates/trace-commons-contributor/src/private_inference_copy.rs` (inline)

**Interfaces:**
- Consumes: `PrivateInferenceTone.fromABI(_:)`, `PrivateInferenceSurface.tone(_:calls:)`.
- Produces: `PrivateInferenceTone.readsAsWorking: Bool` — the single predicate every shell indicator must use.

- [ ] **Step 1: Write the failing Swift test**

```swift
import XCTest
@testable import TCShellCore

final class PrivateInferenceToneTests: XCTestCase {
    func testOnlyClearReadsAsWorking() {
        XCTAssertTrue(PrivateInferenceTone.clear.readsAsWorking)
        for tone: PrivateInferenceTone in [.neutral, .held, .attention, .refused] {
            XCTAssertFalse(
                tone.readsAsWorking,
                "\(tone) must not be painted as working"
            )
        }
    }

    func testUnknownABIValuesDoNotReadAsWorking() {
        for raw: Int32 in [-1, 99, Int32.max, Int32.min] {
            XCTAssertFalse(
                PrivateInferenceTone.fromABI(raw).readsAsWorking,
                "unknown ABI value \(raw) must not be painted as working"
            )
        }
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd macos && swift test --filter PrivateInferenceToneTests`
Expected: FAIL — no member `readsAsWorking`.

- [ ] **Step 3: Add the predicate**

In `PrivateInferenceSurface.swift`, on `PrivateInferenceTone`:

```swift
    /// Whether an indicator may paint this tone as working.
    ///
    /// `Clear` alone. A tab badge and a tray glyph both invite a green dot,
    /// and painting `refused` or `held` as "on" is the fail-open this
    /// surface exists to prevent. Shells must ask this, never the settings
    /// boolean: the switch says what was asked for, this says what is true.
    public var readsAsWorking: Bool { self == .clear }
```

- [ ] **Step 4: Run the test**

Run: `cd macos && swift test --filter PrivateInferenceToneTests`
Expected: PASS.

- [ ] **Step 5: Mirror it in Rust for the GTK shell**

In `private_inference_copy.rs`, on `PrivateInferenceTone`:

```rust
impl PrivateInferenceTone {
    /// Whether an indicator may paint this tone as working. `Clear` alone.
    pub fn reads_as_working(self) -> bool {
        matches!(self, PrivateInferenceTone::Clear)
    }
}
```

With its test:

```rust
#[test]
fn only_clear_reads_as_working() {
    assert!(PrivateInferenceTone::Clear.reads_as_working());
    for tone in [
        PrivateInferenceTone::Neutral,
        PrivateInferenceTone::Held,
        PrivateInferenceTone::Attention,
        PrivateInferenceTone::Refused,
    ] {
        assert!(!tone.reads_as_working(), "{tone:?} must not read as working");
    }
}
```

- [ ] **Step 6: Run both**

Run: `cargo test -p trace-commons-contributor private_inference`
Run: `cd macos && swift test --filter PrivateInferenceToneTests`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add macos/Sources/TCShellCore/PrivateInferenceSurface.swift \
        macos/Tests/TCShellCoreTests/PrivateInferenceToneTests.swift \
        crates/trace-commons-contributor/src/private_inference_copy.rs
git commit -m "Pin the rule that only a clear tone reads as working"
```

---

### Task 3: macOS fifth destination

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift:10-39` (Section enum), `:126` (content switch), `:145`, `:150`, `:262`
- Create: `macos/Sources/TraceCommonsApp/Views/PrivateInferenceView.swift`

**Interfaces:**
- Consumes: Task 1's `destination`/`subtitle`; Task 2's `readsAsWorking`; existing `PrivateInferenceSurface.stateLine/tone/servingLine/settingsParams`.
- Produces: `MainWindowView.Section.privateInference`.

- [ ] **Step 1: Add the case**

In `Section` (line 10). Note `queue` and `compute` both already return `.monitor`; a third reuse would make the sidebar ambiguous, so use a distinct glyph:

```swift
        case privateInference = "privateInference"
```

In `glyph`, add an arm using a glyph not already taken by `queue`/`compute`. In `subtitle`, return `""` — like `compute`, the real subtitle comes from Rust copy.

- [ ] **Step 2: Source the label from Rust**

At line 150 the sidebar label is `item == .compute ? (compute.copy?.destination ?? "") : item.rawValue`. Extend it so `privateInference` also takes its label from copy rather than `rawValue`, and do the same for the subtitle at line 262. Do not hardcode the words in Swift.

- [ ] **Step 3: Build the destination view**

Create `PrivateInferenceView.swift` rendering, from the shared surface only: the title (`copy.settingsTitle`), the state line, the serving line when non-nil, the toggle (`copy.settingsToggle`, calling `PrivateInferenceSurface.settingsParams(on:)`), the "applies at once" line, and `copy.offerExposure` in full.

The indicator must be driven by `PrivateInferenceSurface.tone(state, calls:).readsAsWorking` — never by the toggle's bound boolean.

- [ ] **Step 4: Write the view test**

```swift
func testIndicatorDoesNotFollowTheSwitch() {
    // Switch on, but the daemon reports a refusal: the indicator must not
    // read as working.
    let state = PrivateInferenceState(label: "port_in_use", port: nil)
    let tone = PrivateInferenceSurface.tone(state, calls: .testing)
    XCTAssertFalse(tone.readsAsWorking)
}
```

- [ ] **Step 5: Run**

Run: `cd macos && swift test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/MainWindowView.swift \
        macos/Sources/TraceCommonsApp/Views/PrivateInferenceView.swift \
        macos/Tests/
git commit -m "Give private inference its own destination on macOS"
```

---

### Task 4: macOS menu bar section and shortcuts

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/MenuBarView.swift` (beside `waitingSection` at `:187` and `healthSection` at `:212`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift`

**Interfaces:**
- Consumes: Task 2's `readsAsWorking`, Task 3's `Section.privateInference`.

- [ ] **Step 1: Add a `privateInferenceSection`**

Following the shape of `waitingSection`: the state line as text, and a direct toggle calling `settingsParams(on:)`. State text comes from the surface; the working/not-working glyph from `readsAsWorking`.

- [ ] **Step 2: Add shortcuts**

`Cmd-1..5` for the five destinations, and a toggle shortcut for private inference. These are in-app only — no global hotkey, which is cut 2.

- [ ] **Step 3: Test the menu bar never claims working on a non-clear tone**

```swift
func testMenuBarGlyphFollowsToneNotSwitch() {
    for label in ["port_in_use", "start_failed", "crashed", "stopping", "unknown_state"] {
        let state = PrivateInferenceState(label: label, port: nil)
        XCTAssertFalse(PrivateInferenceSurface.tone(state, calls: .testing).readsAsWorking,
                       "\(label) must not read as working in the menu bar")
    }
}
```

- [ ] **Step 4: Run**

Run: `cd macos && swift test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/MenuBarView.swift \
        macos/Sources/TraceCommonsApp/Views/MainWindowView.swift macos/Tests/
git commit -m "Put private inference in the macOS menu bar and on a shortcut"
```

---

### Task 5: GTK fourth screen

GTK has three screens today (`queue`, `history`, `settings`) — there is no Compute on Linux, so this is 3 → 4. `SCREENS` and `pages` are fixed-size arrays and must grow together.

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/mod.rs:59` (`SCREENS`), `:224` (`pages`), `:9-19` (module list)
- Create: `crates/trace-commons-contributor-gtk/src/ui/private_inference.rs`

**Interfaces:**
- Consumes: Task 1's copy fields; Task 2's `reads_as_working()`.
- Produces: `private_inference::PrivateInferenceView::new() -> PrivateInferenceView` with a `root: gtk::Box`, matching `queue::QueueView` / `settings::SettingsView`.

- [ ] **Step 1: Grow both arrays together**

```rust
const SCREENS: [(&str, &str, &str); 4] = [
    ("queue", "Queue", "view-list-symbolic"),
    ("history", "History", "document-open-recent-symbolic"),
    ("private-inference", private_inference_copy::DESTINATION, "network-transmit-receive-symbolic"),
    ("settings", "Settings", "emblem-system-symbolic"),
];
```

The icon must be **symbolic** — GTK recolours symbolic icons from the node's `color`, and a full-colour icon would silently fail to recolour while the others kept working.

And at line 224:

```rust
        let private_inference = private_inference::PrivateInferenceView::new();
        let pages: [&gtk::Box; 4] =
            [&queue.root, &history.root, &private_inference.root, &settings.root];
```

- [ ] **Step 2: Add the module**

In `ui/mod.rs`'s module list: `pub mod private_inference;`

- [ ] **Step 3: Build the view**

`private_inference.rs` renders the same content as the macOS destination, reading `trace_commons_contributor::private_inference_copy` directly — GTK bypasses the C ABI and uses the Rust crate natively. The indicator uses `reads_as_working()`.

Because GNOME has no tray, this window must carry the full capability; nothing may be reachable only from a tray.

- [ ] **Step 4: Test the arrays cannot drift**

```rust
#[test]
fn every_screen_has_a_page() {
    assert_eq!(SCREENS.len(), 4);
    for (name, label, icon) in SCREENS {
        assert!(!name.is_empty() && !label.is_empty());
        assert!(icon.ends_with("-symbolic"), "{icon} must be symbolic to recolour");
    }
}
```

- [ ] **Step 5: Run**

Run: `RUSTFLAGS="-D warnings" cargo check --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`
Run: `cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`
Expected: PASS. Note the GTK crate is a **separate workspace** with its own lock — a `--workspace` check from the repo root does not cover it.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/ui/mod.rs \
        crates/trace-commons-contributor-gtk/src/ui/private_inference.rs
git commit -m "Give private inference its own screen on Linux"
```

---

### Task 6: GTK tray shortcut

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/tray.rs`

- [ ] **Step 1: Add a tray entry that opens the screen and toggles**

A shortcut into the window, never the only path to the capability — `ui/mod.rs` requires every capability be reachable from the window. State text from the copy module; any glyph from `reads_as_working()`.

- [ ] **Step 2: Run**

Run: `RUSTFLAGS="-D warnings" cargo check --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/tray.rs
git commit -m "Offer private inference from the Linux tray as a shortcut"
```

---

### Task 7: Windows nav item, tray section and shortcuts

**Files:**
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml`, `MainWindow.xaml.cs`
- Modify: `windows/src/TraceCommons.App/TrayIcon.cs`
- Create: `windows/src/TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs`

**Interfaces:**
- Consumes: Task 1's copy fields through `TraceCommons.Interop`.

- [ ] **Step 1: Add the nav item**

A nav item in `MainWindow.xaml` whose label comes from Rust copy through the interop layer — Windows copy comes from Rust, never a XAML literal.

- [ ] **Step 2: Add the view model and page**

Same content as the other two shells. The indicator derives from the tone, exposed through interop; never from the toggle's boolean.

- [ ] **Step 3: Add the tray section**

In `TrayIcon.cs`, state text plus a direct toggle.

- [ ] **Step 4: Add shortcuts**

`Ctrl-1..5` and a toggle accelerator. In-app only.

- [ ] **Step 5: Test the tone mapping**

A test asserting that every non-clear tone, and any unrecognised value, fails to read as working.

- [ ] **Step 6: Run**

Run the Windows contributor app tests (the `windows contributor app` and `windows contributor crate tests` CI jobs cover this).
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add windows/src/TraceCommons.App/
git commit -m "Give private inference a destination and tray entry on Windows"
```

---

### Task 8: Point Settings at the new destination

The Settings entry is not removed — muscle memory and existing instructions depend on it, and a pointer costs nothing.

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/settings.rs`
- Modify: `windows/src/TraceCommons.App/` settings surface
- Modify: `crates/trace-commons-contributor/src/private_inference_copy.rs`

- [ ] **Step 1: Add the pointer sentence to the copy module**

```rust
const SETTINGS_MOVED: &str = "Model calls now has its own place in the sidebar.";
```

Add `pub settings_moved: &'static str` to the struct and constructor, and the matching Swift field and `CodingKeys` case — both sides move together or the field-set test fails.

- [ ] **Step 2: Replace the inline control with the pointer in all three shells**

The control itself now lives in the destination. Settings shows the sentence and a way to get there.

- [ ] **Step 3: Confirm the offer is undisturbed**

Run the existing offer tests. `shouldOffer(answered, on) == !answered && !on` must still hold and the first-run offer must still appear exactly once.

- [ ] **Step 4: Run everything**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins`
Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Run: `cd macos && swift test`
Run: `cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/private_inference_copy.rs \
        macos/Sources/ crates/trace-commons-contributor-gtk/src/ui/settings.rs \
        windows/src/
git commit -m "Point the settings entry at the private inference destination"
```

---

## Self-review

**Spec coverage.** Navigation → Tasks 3, 5, 7. Destination contents → Tasks 3, 5, 7. Tray/menu bar → Tasks 4, 6, 7. Shortcuts → Tasks 4, 7. Settings pointer → Task 8. The tone rule → Task 2, enforced again per shell in 3, 4, 7. Nav copy → Task 1. Distinct macOS glyph → Task 3 Step 1. Cut 2 excluded throughout.

**Placeholders.** None. Task 7's steps are lighter on literal code than the macOS and GTK tasks because the WinUI sources were not read in full while planning; the implementer should read `MainWindow.xaml` and `TrayIcon.cs` and follow the existing nav-item and tray-entry shapes rather than inventing new ones.

**Type consistency.** `destination`/`subtitle` (Task 1) are used in Tasks 3, 5, 7. `readsAsWorking` (Swift) and `reads_as_working()` (Rust) are named per language convention and used consistently. `settingsParams(on:)`, `offerParams(accepted:)`, `PrivateInferenceState(label:port:)` match the existing signatures in `PrivateInferenceSurface.swift`.

**Known ordering constraint.** Task 1 must land before 3, 5 and 7; Task 2 before every shell task. Tasks 3-4 (macOS), 5-6 (GTK) and 7 (Windows) are independent of each other.

---

## Traps found while implementing Tasks 1-2

Any task that adds a copy field must do all four of these, or something fails
loudly (or worse, silently):

1. `every_sentence_arrives_finished` pins the field count -- it moved 22 -> 24.
   Adding a field means updating that assertion.
2. `PrivateInferenceCopy` decoding is **all-or-nothing**, so the sentinel payload
   in `macos/Tests/TCShellCoreTests/PrivateInferenceSurfaceTests.swift` must gain
   every new key or roughly 11 tests fail at once.
3. `docs/contributor-daemon-ipc-v1_1.md:1672` enumerates the payload fields and
   states the count. It is **not** test-enforced, so it goes stale silently.
4. **Windows does NOT tolerate a new key -- adding one turns the Windows crate
   tests red immediately.** An earlier version of this note said otherwise and was
   wrong. `PrivateInferenceCopy.cs` has no `JsonUnmappedMemberHandling.Disallow`
   and no field-count assertion, which is what that reading checked -- but
   `windows/tests/TraceCommons.Interop.Tests/PrivateInferenceTests.cs:52`,
   `EveryExportedFieldIsDecodedAndNoneIsInvented`, asserts set equality in BOTH
   directions. Adding a field to the Rust export without adding the C# property
   fails it. The Windows shell must add the properties to stay **green**, not
   merely to consume them, and the two sides have to land together or the branch
   is red in between.

GTK needs nothing extra for the tone predicate:
`crates/trace-commons-contributor-gtk/src/copy.rs:2094` re-exports from
`private_inference_copy` directly, so `reads_as_working()` is already reachable.

5. **`ShellWordingTests` is a hard per-file ratchet, and Task 8 will hit it.**
   `macos/Tests/TCShellCoreTests/ShellWordingTests.swift` (mirrored by
   `ShellWordingTests.cs` on Windows) counts sentence-shaped string literals per
   file and holds each count as **a CEILING AND A FLOOR both**: adding a sentence
   fails, and *removing* one fails too. "Never raise a number. A new file must
   never be added here."

   Consequences: any NEW view file must author **zero** sentence literals -- take
   every word from the copy payload. And Task 8, which removes the inline control
   from `SettingsView.swift`, will push that file BELOW its recorded count and
   fail until the baseline entry is lowered deliberately, in the same commit.
   That is the intended workflow, not a broken test: the number is meant to
   ratchet down as copy moves behind the ABI.

   The numbers are measured, not typed -- regenerate with `TC_WORDING_DUMP=1`
   rather than guessing.
