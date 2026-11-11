#!/usr/bin/env python3
"""Build-profile identity and bundle agreement; uses no network."""
import argparse
import ctypes
import hashlib
import json
import pathlib
import plistlib
import re
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]

def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result

def identity(channel, selected=None, root=ROOT):
    selected = selected or channel
    if (channel, selected) not in [("production", "production"), ("staging", "staging"), ("staging", "testing")]:
        raise ValueError("feature-profile-channel-mismatch")
    source = root / "crates/trace-commons-contributor/feature-flags"
    profile = json.loads((source / "profiles" / (selected + ".json")).read_text(), object_pairs_hook=unique)
    registry = json.loads((source / "registry.json").read_text(), object_pairs_hook=unique)
    # Field order of the Rust `Profile`, whose serialization the digest covers.
    keys = ["schema_version", "registry_version", "environment", "capabilities", "remote_enable", "signing_keys"]
    sets = ["capabilities", "remote_enable", "signing_keys"]
    if set(profile) != set(keys) or profile["schema_version"] != 1 or registry["schema_version"] != 1 or profile["registry_version"] != registry["registry_version"] or profile["environment"] != selected:
        raise ValueError("feature-profile-invalid")
    if not all(isinstance(profile[k], list) and all(isinstance(v, str) for v in profile[k]) for k in sets):
        raise ValueError("feature-profile-set-invalid")
    if any(k not in registry["flags"] for k in profile["remote_enable"]):
        raise ValueError("unknown-profile-flag")
    check_remote_enables(registry, profile)
    canonical = {k: sorted(set(profile[k])) if k in sets else profile[k] for k in keys}
    digest = hashlib.sha256(json.dumps(canonical, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    return {"schema_version": 1, "environment": selected, "profile_digest": digest}

def check_remote_enables(registry, profile):
    """The same guards as the Rust `Registry::validate`, without a Rust build."""
    for definition in registry["flags"].values():
        if definition.get("class") not in ("product", "privacy_sensitive"):
            raise ValueError("feature-flag-class-missing")
    if not all(re.fullmatch(r"[0-9a-f]{64}", key) for key in profile["signing_keys"]):
        raise ValueError("invalid-signing-key")
    if profile["remote_enable"] and not profile["signing_keys"]:
        raise ValueError("remote-enable-requires-signing-key")
    for flag in profile["remote_enable"]:
        if registry["flags"][flag]["class"] == "privacy_sensitive":
            raise ValueError("privacy-sensitive-flag-remotely-enabled")
        if registry["flags"][flag]["default"] is not False:
            raise ValueError("remotely-enabled-flag-defaults-on")

def verify_values(expected, cli, library, plist):
    bundled = {"schema_version": 1, "environment": plist.get("TCFeatureEnvironment"), "profile_digest": plist.get("TCFeatureProfileDigest")}
    if cli != expected or library != expected or bundled != expected:
        raise ValueError("bundle-feature-profile-mismatch")

def verify(app, channel, profile):
    expected = identity(channel, profile)
    cli = json.loads(subprocess.check_output([str(app / "Contents/MacOS/near-ai"), "feature-flags", "build-profile", "--json"], text=True))
    with tempfile.TemporaryDirectory() as directory:
        copied = pathlib.Path(directory) / "lib.dylib"
        shutil.copyfile(app / "Contents/Frameworks/libtrace_commons_contributor_ffi.dylib", copied)
        if shutil.which("codesign"):
            subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(copied)], check=True, capture_output=True)
        lib = ctypes.CDLL(str(copied))
        lib.tc_feature_profile_json.restype = ctypes.c_void_p
        lib.tc_string_free.argtypes = [ctypes.c_void_p]
        raw = lib.tc_feature_profile_json()
        if not raw:
            raise ValueError("missing-library-feature-profile")
        try:
            library = json.loads(ctypes.string_at(raw))
        finally:
            lib.tc_string_free(raw)
    with (app / "Contents/Info.plist").open("rb") as stream:
        plist = plistlib.load(stream)
    verify_values(expected, cli, library, plist)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", required=True, choices=["production", "staging"])
    parser.add_argument("--profile")
    parser.add_argument("--app", type=pathlib.Path)
    args = parser.parse_args()
    try:
        if args.app:
            verify(args.app, args.channel, args.profile)
        else:
            print(json.dumps(identity(args.channel, args.profile)))
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        parser.error(str(error))

if __name__ == "__main__":
    main()
