#!/usr/bin/env python3
"""Validate and publish a campaign without building or restarting the app."""
import argparse
import json
import pathlib
import subprocess
import sys
import unicodedata
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[2]
DEFAULT = ROOT / "crates/trace-commons-contributor/src/private_ai_campaign.json"
TARGETS = {
    "staging": "gs://tc-staging-updates-20261022/staging/private-ai-campaign.json",
    "production": "gs://tracecommons-flatpak/updates/private-ai-campaign.json",
}


def validate(path, target):
    raw = path.read_bytes()
    if len(raw) > 16384:
        raise ValueError("campaign exceeds 16 KiB")
    item = json.loads(raw)
    if type(item.get("schema_version")) is not int or item["schema_version"] != 1:
        raise ValueError("schema_version must be 1")
    for key, maximum in [("id", 100), ("title", 100), ("body", 300), ("action_title", 40)]:
        value = item.get(key)
        if (not isinstance(value, str) or not value.strip() or len(value) > maximum
                or "<" in value or ">" in value
                or any(unicodedata.category(c) in ["Cc", "Cf"] for c in value)):
            raise ValueError(f"{key} must be nonempty text of at most {maximum} characters")
    url = urllib.parse.urlsplit(item.get("action_url", ""))
    if (url.scheme != "https" or url.hostname != "near.ai" or url.username is not None
            or url.password is not None or url.port not in [None, 443] or url.fragment):
        raise ValueError("action_url must be an HTTPS near.ai URL")
    if item.get("symbol") not in ["cpu", "sparkles", "bolt", "globe"]:
        raise ValueError("symbol must be cpu, sparkles, bolt or globe")
    image = item.get("image_url")
    if image is not None:
        u = urllib.parse.urlsplit(image)
        directory = "https://storage.googleapis.com/" + target.removeprefix("gs://").rsplit("/", 1)[0] + "/"
        decoded_path = urllib.parse.unquote(u.path)
        expected_directory = urllib.parse.urlsplit(directory).path.rstrip("/")
        if (not image.startswith(directory) or decoded_path.rsplit("/", 1)[0] != expected_directory
                or "%" in decoded_path or "\\" in decoded_path
                or "%2f" in u.path.lower() or "%5c" in u.path.lower()
                or u.username is not None or u.password is not None
                or u.query or u.fragment or u.port not in [None, 443]
                or not u.path.lower().endswith((".png", ".jpg", ".jpeg"))
                or any(part in [".", ".."] for part in urllib.parse.unquote(u.path).split("/"))):
            raise ValueError("image_url must be a PNG or JPEG in this campaign directory")
    return item


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", choices=TARGETS, required=True)
    parser.add_argument("--file", type=pathlib.Path, default=DEFAULT)
    parser.add_argument("--account", help="An already authenticated gcloud account; does not change defaults")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    target = TARGETS[args.channel]
    try:
        item = validate(args.file, target)
    except (OSError, ValueError, TypeError, AttributeError) as error:
        parser.error(str(error))
    print(f"{item['id']} -> {target}", flush=True)
    command = ["gcloud", "storage", "cp", "--content-type=application/json",
               "--cache-control=public,max-age=300", str(args.file), target]
    if args.account:
        command.append("--account=" + args.account)
    if not args.dry_run:
        return subprocess.run(command, check=False).returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
