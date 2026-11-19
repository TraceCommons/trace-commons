#!/usr/bin/env python3
"""Pair macOS and Linux window captures into one self-contained HTML page.

Slice 1 of docs/superpowers/specs/2026-11-16-linux-mac-window-and-map-design.md.
Standard library only. Every capture is named

    <set>-<tab>-<panes>-<appearance>.png

where <set> is a recorded sample set (letters only, e.g. normalDay), <tab> is
home, inference, traces or insights, <panes> is main, main+map,
main+inspector or main+map+inspector, and <appearance> is light or dark. The
macOS hook (macos/Sources/TraceCommonsApp/DebugScreenshot.swift) writes these
names; a Linux capture must use the same ones.

The second side is named Linux unless PARITY_SIDE names it (Windows for the
Windows captures).

Usage: gallery.py <mac-dir> <linux-dir> <out.html>
"""

import base64
import html
import os
import re
import sys
from typing import List, NamedTuple, Optional, Tuple

TABS = ("home", "inference", "traces", "insights")
NAME = re.compile(
    r"^(?P<set>[A-Za-z]+)-(?P<tab>home|inference|traces|insights)"
    r"-(?P<panes>main(?:\+map)?(?:\+inspector)?)-(?P<appearance>light|dark)\.png$"
)


class Key(NamedTuple):
    set: str
    tab: str
    panes: str
    appearance: str

    def filename(self) -> str:
        return f"{self.set}-{self.tab}-{self.panes}-{self.appearance}.png"

    def order(self):
        return (self.set, TABS.index(self.tab), self.panes, self.appearance)


class Pair(NamedTuple):
    key: Key
    mac: str
    linux: str


class Result(NamedTuple):
    pairs: List[Pair]
    mac_only: List[Key]
    linux_only: List[Key]
    unparsed: List[Tuple[str, str]]
    mac_dir: str
    linux_dir: str


def parse(name: str) -> Optional[Key]:
    match = NAME.match(name)
    if not match:
        return None
    return Key(match["set"], match["tab"], match["panes"], match["appearance"])


def _scan(directory: str, side: str, unparsed: List[Tuple[str, str]]) -> dict:
    found = {}
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if not os.path.isfile(path):
            continue
        key = parse(name)
        if key is None:
            unparsed.append((side, name))
        else:
            found[key] = path
    return found


def collect(mac_dir: str, linux_dir: str) -> Result:
    unparsed: List[Tuple[str, str]] = []
    mac = _scan(mac_dir, "macos", unparsed)
    linux = _scan(linux_dir, "linux", unparsed)
    order = Key.order
    pairs = [Pair(k, mac[k], linux[k]) for k in sorted(mac.keys() & linux.keys(), key=order)]
    return Result(
        pairs=pairs,
        mac_only=sorted(mac.keys() - linux.keys(), key=order),
        linux_only=sorted(linux.keys() - mac.keys(), key=order),
        unparsed=unparsed,
        mac_dir=mac_dir,
        linux_dir=linux_dir,
    )


def _img(path: str, alt: str) -> str:
    with open(path, "rb") as f:
        data = base64.b64encode(f.read()).decode("ascii")
    return f'<img alt="{html.escape(alt)}" src="data:image/png;base64,{data}">'


def side_name(environ) -> str:
    """The second side's name: PARITY_SIDE, or Linux."""
    return environ.get("PARITY_SIDE") or "Linux"


def render(result: Result, side: str = "Linux") -> str:
    out = [
        "<!doctype html>",
        '<html lang="en"><head><meta charset="utf-8">',
        '<meta name="viewport" content="width=device-width, initial-scale=1">',
        f"<title>Mac and {html.escape(side)} captures</title>",
        "<style>",
        ":root{--bg:#f6f6f7;--fg:#1d1d1f;--muted:#6e6e73;--rule:#d2d2d7}",
        "@media (prefers-color-scheme: dark){:root{--bg:#1c1c1e;--fg:#f5f5f7;--muted:#a1a1a6;--rule:#3a3a3c}}",
        "body{margin:0;padding:16px;background:var(--bg);color:var(--fg);font:14px/1.4 system-ui,sans-serif}",
        "section{border-top:1px solid var(--rule);padding:12px 0}",
        ".row{display:grid;grid-template-columns:1fr 1fr;gap:12px}",
        "figure{margin:0;min-width:0}figcaption{color:var(--muted);font-size:12px}",
        "img{max-width:100%;height:auto;border:1px solid var(--rule)}",
        "@media (max-width:700px){.row{grid-template-columns:1fr}}",
        "</style></head><body>",
        f"<h1>Mac and {html.escape(side)} captures</h1>",
        f"<p>{len(result.pairs)} paired, {len(result.mac_only)} macOS only, "
        f"{len(result.linux_only)} {html.escape(side)} only, {len(result.unparsed)} not named to the grammar.</p>",
    ]
    for pair in result.pairs:
        name = pair.key.filename()
        out += [
            f"<section><h2>{html.escape(name)}</h2><div class=\"row\">",
            f"<figure>{_img(pair.mac, 'macOS ' + name)}<figcaption>macOS</figcaption></figure>",
            f"<figure>{_img(pair.linux, side + ' ' + name)}<figcaption>{html.escape(side)}</figcaption></figure>",
            "</div></section>",
        ]
    for label, keys, directory in (
        ("macOS only", result.mac_only, result.mac_dir),
        (f"{side} only", result.linux_only, result.linux_dir),
    ):
        if not keys:
            continue
        out.append(f"<section><h2>{html.escape(label)}</h2>")
        for key in keys:
            name = key.filename()
            path = os.path.join(directory, name)
            out.append(
                f"<figure>{_img(path, label + ' ' + name)}<figcaption>{html.escape(name)}</figcaption></figure>"
            )
        out.append("</section>")
    if result.unparsed:
        out.append("<section><h2>Not named to the grammar</h2><ul>")
        for side, name in result.unparsed:
            out.append(f"<li>{html.escape(side)}: {html.escape(name)}</li>")
        out.append("</ul></section>")
    out.append("</body></html>")
    return "\n".join(out) + "\n"


def main(argv: List[str]) -> int:
    if len(argv) != 4:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    mac_dir, linux_dir, out_path = argv[1:]
    for directory in (mac_dir, linux_dir):
        if not os.path.isdir(directory):
            print(f"not a directory: {directory}", file=sys.stderr)
            return 2
    side = side_name(os.environ)
    result = collect(mac_dir, linux_dir)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write(render(result, side))
    print(
        f"{out_path}: {len(result.pairs)} paired, {len(result.mac_only)} macOS only, "
        f"{len(result.linux_only)} {side} only, {len(result.unparsed)} unparsed"
    )
    for key in result.mac_only:
        print(f"macOS only: {key.filename()}")
    for key in result.linux_only:
        print(f"{side} only: {key.filename()}")
    for side, name in result.unparsed:
        print(f"unparsed ({side}): {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
