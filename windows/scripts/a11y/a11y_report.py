#!/usr/bin/env python3
"""Reads one W25 audit run (win-a11y-run.ps1 output, copied off the box) and
fails on what the one-pass checklist forbids (items E1, E2 and D1).

    python3 a11y_report.py <results-dir> <tag>

For each surface the run walked with Tab:
  - silent stop: a stop whose UI Automation name is empty (E1);
  - unseen stop: a press after which nothing on screen changed, or whose
    element is off screen (E2: focus visible);
  - silent start: a window that opens with its bare input site focused;
  - trap: focus that did not move for three presses;
  - a name holding "Ctrl+", "Trace Commons" or "Near.ai".
From every UIA dump: an enabled, on-screen, keyboard-focusable element with no
name. From every contrast file (measured on the box from the drawn pixels):
named text under 4.5:1, or a control's label under 3:1 (a label with no Text
element may be a glyph, held to WCAG 1.4.11's 3:1; 3 to 4.5 is printed as
WARN, to be looked at in the capture). From the driver's log: every EXPECT
FAIL.

A rectangle with four colours or fewer, or under 9 px tall, drew nothing (a
label clipped out of its viewport, which UI Automation still reports) and is
not counted. Exit code 1 when anything fails.

Standard library only.
"""
import glob
import os
import re
import sys

BANNED = ("Ctrl+", "Trace Commons", "Near.ai")
STOP = re.compile(r'\s*(\d+|start): (\S+) \| (\w+) \| class=([^|]*) \| aid=([^|]*) \| name="([^"]*)"')


def stop(line):
    m = STOP.match(line)
    if not m:
        return None
    changed = re.search(r"changed=(-?\d+)", line)
    offscreen = re.search(r"offscreen=(\w+)", line)
    return {
        "i": m.group(1), "rid": m.group(2), "type": m.group(3), "cls": m.group(4).strip(),
        "aid": m.group(5).strip(), "name": m.group(6),
        "changed": int(changed.group(1)) if changed else None,
        "offscreen": offscreen is not None and offscreen.group(1) == "True",
    }


def surface(path, tag, suffix):
    return os.path.basename(path)[len(tag) + 1:-len(suffix)]


def main(results, tag):
    failures = []
    for path in sorted(glob.glob(os.path.join(results, f"{tag}-*.tab.txt"))):
        name = surface(path, tag, ".tab.txt")
        lines = open(path, encoding="utf-8-sig").read().splitlines()
        stops = [s for s in (stop(l) for l in lines if re.match(r"\s*\d+:", l)) if s]
        end = next((l for l in lines if l.startswith(("CYCLE", "TRAP"))), "NO CYCLE")
        count = int(re.search(r"\d+", end).group()) if end.startswith("CYCLE") else len(stops)
        walk = stops[:count]
        start = next((stop(l) for l in lines if l.startswith("start:")), None)
        silent = [s for s in walk if not s["name"].strip()]
        # A window with one stop cannot move its focus: its press changes
        # nothing and that is not an unseen stop.
        single = count == 1 and start is not None and walk and walk[0]["rid"] == start["rid"]
        unseen = [s for s in walk if (s["changed"] == 0 and not single) or s["offscreen"]]
        banned = [s for s in walk if any(b.lower() in s["name"].lower() for b in BANNED)]
        print(f"{name}: {count} stops, {len(silent)} silent, {len(unseen)} unseen ({end.split()[0]})")
        for s in silent:
            failures.append(f"{name}: silent stop {s['i']} ({s['type']} {s['cls']} {s['aid']})")
        for s in unseen:
            failures.append(f"{name}: unseen stop {s['i']} '{s['name']}' changed={s['changed']} offscreen={s['offscreen']}")
        for s in banned:
            failures.append(f"{name}: banned word in '{s['name']}'")
        if start and not start["name"].strip() and start["type"] == "Pane":
            failures.append(f"{name}: opens with its bare window focused ({start['cls']})")
        if end.startswith("TRAP"):
            failures.append(f"{name}: {end}")

    for path in sorted(glob.glob(os.path.join(results, f"{tag}-*.uia.txt"))):
        name = surface(path, tag, ".uia.txt")
        for line in open(path, encoding="utf-8-sig"):
            if "focusable=True" in line and "enabled=True" in line and "offscreen=False" in line:
                m = re.search(r'(\w+) \| rid=\S+ \| name="([^"]*)".*aid=([^|]*)\| class=([^|]*)', line)
                if m and not m.group(2).strip() and m.group(1) not in ("Window", "Pane"):
                    failures.append(f"{name}: unnamed focusable {m.group(1)} class={m.group(4).strip()} aid={m.group(3).strip()}")

    seen = set()
    for path in sorted(glob.glob(os.path.join(results, f"{tag}-*.contrast.txt"))):
        name = surface(path, tag, ".contrast.txt")
        for line in open(path, encoding="utf-8-sig"):
            m = re.search(r'\| (\w+) \| name="([^"]*)" \| rect=[^|]*?x(\d+) \| bg=#(\w+) ink=#(\w+) ratio=([\d.]+).*colours=(\d+)', line)
            if not m or not m.group(2).strip() or int(m.group(7)) <= 4 or int(m.group(3)) < 9:
                continue
            # Text is held to 4.5:1. A control's label without a Text element
            # may be words or a glyph, which UI Automation cannot tell apart:
            # under 3:1 (WCAG 1.4.11, graphics) fails, 3 to 4.5 is printed as
            # WARN for a look at the capture.
            text = m.group(1) in ("Text", "Hyperlink")
            ratio = float(m.group(6))
            key = (re.sub(r"-sc\d+-\d+$", "", name), m.group(2)[:60], m.group(4), m.group(5))
            if key in seen or ratio >= 4.5:
                continue
            seen.add(key)
            line = f"{name}: {m.group(6)}:1 {m.group(1)} '{m.group(2)[:60]}' ink #{m.group(5)} on #{m.group(4)}"
            if text or ratio < 3.0:
                failures.append(line)
            else:
                print("WARN " + line)

    log = os.path.join(results, f"{tag}.drive.log")
    if os.path.exists(log):
        for line in open(log, encoding="utf-8-sig"):
            if "EXPECT FAIL" in line or "FAILED:" in line:
                failures.append(line.strip())
            elif "EXPECT PASS" in line:
                print(line.strip())

    for failure in failures:
        print("FAIL " + failure)
    print(f"{tag}: {len(failures)} failures")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
