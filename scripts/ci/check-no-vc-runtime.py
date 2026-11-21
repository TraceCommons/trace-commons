#!/usr/bin/env python3
"""Fail if a Windows PE file imports the Visual C++ runtime.

A stock Windows install has the Universal CRT (ucrtbase.dll, reached through
the api-ms-win-crt-* API sets) but not the Visual C++ runtime
(vcruntime140.dll, msvcp140.dll and friends), which only arrives with the
Visual C++ Redistributable. The MSIX declares no VCLibs dependency and bundles
no copy, so a shipped binary that imports one fails to load on a clean
machine: for the contributor FFI cdylib that is a DllNotFoundException the
first time the app calls into its core.

The Rust builds avoid it with `-C target-feature=+crt-static`. This check
keeps that from regressing.

Usage: python -I scripts/ci/check-no-vc-runtime.py FILE [FILE ...]
Standard library only.
"""

import struct
import sys

FORBIDDEN_PREFIXES = ("vcruntime", "msvcp", "concrt", "vccorlib", "ucrtbased")


def imports(path):
    with open(path, "rb") as f:
        b = f.read()
    if b[:2] != b"MZ":
        raise ValueError(f"{path}: not a PE file")
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    if b[pe : pe + 4] != b"PE\0\0":
        raise ValueError(f"{path}: no PE signature")
    nsec = struct.unpack_from("<H", b, pe + 6)[0]
    opt_size = struct.unpack_from("<H", b, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from("<H", b, opt)[0]
    data_dirs = opt + (112 if magic == 0x20B else 96)
    import_rva = struct.unpack_from("<I", b, data_dirs + 8)[0]
    sections = []
    s = opt + opt_size
    for _ in range(nsec):
        vsize, va, raw_size, raw = struct.unpack_from("<IIII", b, s + 8)
        sections.append((va, max(vsize, raw_size), raw))
        s += 40

    def offset(rva):
        for va, size, raw in sections:
            if va <= rva < va + size:
                return rva - va + raw
        raise ValueError(f"{path}: RVA {rva:#x} is in no section")

    names = []
    if import_rva == 0:
        return names
    o = offset(import_rva)
    while True:
        name_rva = struct.unpack_from("<I", b, o + 12)[0]
        if name_rva == 0:
            break
        n = offset(name_rva)
        names.append(b[n : b.index(b"\0", n)].decode("ascii"))
        o += 20
    return names


def main(paths):
    if not paths:
        print("usage: check-no-vc-runtime.py FILE [FILE ...]", file=sys.stderr)
        return 2
    failed = False
    for path in paths:
        names = imports(path)
        bad = sorted({n for n in names if n.lower().startswith(FORBIDDEN_PREFIXES)})
        # An empty import table would make this check pass vacuously.
        if not any(n.lower() == "kernel32.dll" for n in names):
            print(f"{path}: does not import kernel32.dll; the parser is wrong", file=sys.stderr)
            failed = True
        elif bad:
            print(
                f"{path}: imports the Visual C++ runtime ({', '.join(bad)}); "
                "build with RUSTFLAGS containing -C target-feature=+crt-static",
                file=sys.stderr,
            )
            failed = True
        else:
            print(f"{path}: no Visual C++ runtime imports ({len(names)} DLLs imported)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
