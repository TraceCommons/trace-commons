#!/usr/bin/env python3
"""Tests for `mint-signed-token.py`.

Runs under pytest:

    pytest scripts/operator/test_mint_signed_token.py

or standalone:

    python3 scripts/operator/test_mint_signed_token.py

Exits non-zero on any failed assertion.

The load-bearing test is `test_signature_verifies_against_public_key`: it mints
with a throwaway Ed25519 key and verifies the signature with openssl against the
matching public key. A token this tool produces is worthless if a verifier
rejects it, and nothing else here would catch that.
"""

from __future__ import annotations

import base64
import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("mint-signed-token.py")
_spec = importlib.util.spec_from_file_location("mint_signed_token", MODULE_PATH)
mint_signed_token = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mint_signed_token)


def b64url_decode(segment: str) -> bytes:
    padding = "=" * (-len(segment) % 4)
    return base64.urlsafe_b64decode(segment + padding)


def generate_keypair(directory: Path) -> tuple[Path, Path]:
    private_key = directory / "test-signing.pem"
    public_key = directory / "test-signing.pub.pem"
    subprocess.run(
        ["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(private_key)],
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["openssl", "pkey", "-in", str(private_key), "-pubout", "-out", str(public_key)],
        check=True,
        capture_output=True,
    )
    return private_key, public_key


def test_signature_verifies_against_public_key() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        tmpdir = Path(tmp)
        private_key, public_key = generate_keypair(tmpdir)
        token = mint_signed_token.mint(
            key_path=private_key,
            kid="test-kid",
            tenant="tenant-test",
            role="export_worker",
            issuer="https://issuer.example",
            audience="trace-commons-ingest",
            ttl_seconds=900,
            principal_ref="operator-test",
        )
        header_b64, payload_b64, signature_b64 = token.split(".")
        signing_input = f"{header_b64}.{payload_b64}".encode("ascii")

        message = tmpdir / "message"
        message.write_bytes(signing_input)
        signature = tmpdir / "signature"
        signature.write_bytes(b64url_decode(signature_b64))

        result = subprocess.run(
            [
                "openssl",
                "pkeyutl",
                "-verify",
                "-rawin",
                "-pubin",
                "-inkey",
                str(public_key),
                "-sigfile",
                str(signature),
                "-in",
                str(message),
            ],
            capture_output=True,
        )
        assert result.returncode == 0, (
            "openssl rejected the signature: "
            + result.stderr.decode("utf-8", "replace")
        )


def test_header_carries_eddsa_and_kid() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        private_key, _ = generate_keypair(Path(tmp))
        token = mint_signed_token.mint(
            key_path=private_key,
            kid="kid-abc",
            tenant="tenant-test",
            role="reviewer",
            issuer=None,
            audience=None,
            ttl_seconds=60,
            principal_ref="operator-test",
        )
        header = json.loads(b64url_decode(token.split(".")[0]))
        assert header["alg"] == "EdDSA"
        assert header["typ"] == "JWT"
        assert header["kid"] == "kid-abc"


def test_payload_carries_required_claims() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        private_key, _ = generate_keypair(Path(tmp))
        token = mint_signed_token.mint(
            key_path=private_key,
            kid="kid-abc",
            tenant="tenant-zaki-pilot",
            role="export_worker",
            issuer="https://issuer.tracecommons.ai",
            audience="trace-commons-ingest",
            ttl_seconds=900,
            now=1_700_000_000,
            jti="fixed-jti",
            principal_ref="near-benchmark-handoff-operator",
        )
        payload = json.loads(b64url_decode(token.split(".")[1]))
        assert payload["tenant_id"] == "tenant-zaki-pilot"
        assert payload["role"] == "export_worker"
        assert payload["principal_ref"] == "near-benchmark-handoff-operator"
        assert payload["iss"] == "https://issuer.tracecommons.ai"
        assert payload["aud"] == "trace-commons-ingest"
        assert payload["jti"] == "fixed-jti"
        assert payload["iat"] == 1_700_000_000
        assert payload["exp"] == 1_700_000_900


def test_jti_is_unique_per_mint() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        private_key, _ = generate_keypair(Path(tmp))
        jtis = set()
        for _ in range(3):
            token = mint_signed_token.mint(
                key_path=private_key,
                kid="kid-abc",
                tenant="tenant-test",
                role="admin",
                issuer=None,
                audience=None,
                ttl_seconds=60,
                principal_ref="operator-test",
            )
            jtis.add(json.loads(b64url_decode(token.split(".")[1]))["jti"])
        assert len(jtis) == 3, "jti must be unique per mint; replay protection depends on it"


def test_no_padding_in_segments() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        private_key, _ = generate_keypair(Path(tmp))
        token = mint_signed_token.mint(
            key_path=private_key,
            kid="kid-abc",
            tenant="tenant-test",
            role="admin",
            issuer=None,
            audience=None,
            ttl_seconds=60,
            principal_ref="operator-test",
        )
        assert "=" not in token, "JWS segments must be unpadded base64url"


def test_missing_key_is_reported_not_crashed() -> None:
    try:
        mint_signed_token.mint(
            key_path=Path("/nonexistent/nope.pem"),
            kid="kid-abc",
            tenant="tenant-test",
            role="admin",
            issuer=None,
            audience=None,
            ttl_seconds=60,
            principal_ref="operator-test",
        )
    except mint_signed_token.MintError as error:
        assert "signing key not found" in str(error)
    else:
        raise AssertionError("expected MintError for a missing key")


def test_rejects_missing_principal_and_sub() -> None:
    """The server returns 403 without one of these, so refuse to mint a dud."""
    with tempfile.TemporaryDirectory() as tmp:
        private_key, _ = generate_keypair(Path(tmp))
        try:
            mint_signed_token.mint(
                key_path=private_key,
                kid="kid-abc",
                tenant="tenant-test",
                role="export_worker",
                issuer=None,
                audience=None,
                ttl_seconds=60,
            )
        except mint_signed_token.MintError as error:
            assert "principal_ref or sub" in str(error)
        else:
            raise AssertionError("expected MintError when neither claim is supplied")


def test_sub_satisfies_the_actor_requirement() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        private_key, _ = generate_keypair(Path(tmp))
        token = mint_signed_token.mint(
            key_path=private_key,
            kid="kid-abc",
            tenant="tenant-test",
            role="export_worker",
            issuer=None,
            audience=None,
            ttl_seconds=60,
            subject="operator@example",
        )
        payload = json.loads(b64url_decode(token.split(".")[1]))
        assert payload["sub"] == "operator@example"
        assert "principal_ref" not in payload


def test_rejects_non_positive_ttl() -> None:
    try:
        mint_signed_token.mint(
            key_path=Path("/unused.pem"),
            kid="kid-abc",
            tenant="tenant-test",
            role="admin",
            issuer=None,
            audience=None,
            ttl_seconds=0,
        )
    except mint_signed_token.MintError as error:
        assert "ttl-seconds" in str(error)
    else:
        raise AssertionError("expected MintError for a zero TTL")


def main() -> int:
    failures = 0
    for name, function in sorted(globals().items()):
        if not name.startswith("test_") or not callable(function):
            continue
        try:
            function()
        except Exception as error:  # noqa: BLE001 - standalone runner reports everything
            print(f"FAIL {name}: {error}")
            failures += 1
        else:
            print(f"ok   {name}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
