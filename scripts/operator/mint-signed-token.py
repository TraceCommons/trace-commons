#!/usr/bin/env python3
"""Mint an EdDSA (Ed25519) signed token for a Trace Commons deployment.

The pilot sets `TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS=true` and carries no
`TRACE_COMMONS_TENANT_TOKENS`, so static bearer tokens are rejected before any
handler runs. Every operator surface that gates on a role — reviewer, admin,
export worker — needs a signed token instead. This is that tool.

The claim set is `TraceCommonsSignedTokenClaims` in
`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`: `tenant_id` and
`exp` are required, and the deployment additionally requires `jti`, `iss`, and
`aud` when the corresponding checks are configured.

Signing shells out to `openssl pkeyutl -sign -rawin`, which handles Ed25519
natively in OpenSSL 3.x. That avoids adding a Python cryptography dependency
for a script whose whole job is one signature.

The `kid` must match a key published by the issuer's keyset endpoint, e.g.
`https://issuer.tracecommons.ai/.well-known/trace-commons-ed25519-keyset.json`.

Example:

    python3 scripts/operator/mint-signed-token.py \\
      --key /etc/tracecommons/issuer-signing-v1.pem \\
      --kid 375595e4-fa5c-41be-b35a-3b5ea44e01d3 \\
      --tenant tenant-zaki-pilot \\
      --role export_worker \\
      --issuer https://issuer.tracecommons.ai \\
      --audience trace-commons-ingest \\
      --ttl-seconds 900

Keep the TTL short. The token is a bearer credential: anything holding it can
act in the named role until it expires, and there is no revocation short of
rotating the signing key.
"""

from __future__ import annotations

import argparse
import base64
import json
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path


class MintError(Exception):
    """A minting precondition failed."""


def b64url(raw: bytes) -> str:
    """Base64url-encode without padding, per JWS."""
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")


def build_header(kid: str) -> dict:
    return {"alg": "EdDSA", "typ": "JWT", "kid": kid}


def build_payload(
    tenant: str,
    role: str,
    issuer: str | None,
    audience: str | None,
    ttl_seconds: int,
    now: int,
    jti: str,
    principal_ref: str | None = None,
    subject: str | None = None,
) -> dict:
    payload = {
        "tenant_id": tenant,
        "role": role,
        "jti": jti,
        "iat": now,
        "exp": now + ttl_seconds,
    }
    if issuer:
        payload["iss"] = issuer
    if audience:
        payload["aud"] = audience
    if principal_ref:
        payload["principal_ref"] = principal_ref
    if subject:
        payload["sub"] = subject
    return payload


def sign_ed25519(key_path: Path, signing_input: bytes) -> bytes:
    """Sign with Ed25519 via openssl. Raises MintError on any failure."""
    if not key_path.exists():
        raise MintError(f"signing key not found: {key_path}")
    with tempfile.TemporaryDirectory() as tmp:
        message = Path(tmp) / "message"
        message.write_bytes(signing_input)
        result = subprocess.run(
            [
                "openssl",
                "pkeyutl",
                "-sign",
                "-rawin",
                "-inkey",
                str(key_path),
                "-in",
                str(message),
            ],
            capture_output=True,
        )
    if result.returncode != 0:
        raise MintError(
            "openssl failed to sign: " + result.stderr.decode("utf-8", "replace").strip()
        )
    if len(result.stdout) != 64:
        raise MintError(
            f"expected a 64-byte Ed25519 signature, got {len(result.stdout)} bytes; "
            "is the key actually Ed25519?"
        )
    return result.stdout


def mint(
    key_path: Path,
    kid: str,
    tenant: str,
    role: str,
    issuer: str | None,
    audience: str | None,
    ttl_seconds: int,
    now: int | None = None,
    jti: str | None = None,
    principal_ref: str | None = None,
    subject: str | None = None,
) -> str:
    if ttl_seconds <= 0:
        raise MintError("ttl-seconds must be positive")
    if not (principal_ref or subject):
        raise MintError(
            "a principal_ref or sub claim is required; the server rejects a signed "
            "tenant token without one, and the audit trail derives the actor from it"
        )
    now = int(time.time()) if now is None else now
    jti = jti or str(uuid.uuid4())
    header = build_header(kid)
    payload = build_payload(
        tenant, role, issuer, audience, ttl_seconds, now, jti, principal_ref, subject
    )
    signing_input = ".".join(
        (
            b64url(json.dumps(header, separators=(",", ":")).encode()),
            b64url(json.dumps(payload, separators=(",", ":")).encode()),
        )
    ).encode("ascii")
    signature = sign_ed25519(key_path, signing_input)
    return signing_input.decode("ascii") + "." + b64url(signature)


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--key", required=True, type=Path, help="Ed25519 private key PEM")
    parser.add_argument("--kid", required=True, help="key id published by the issuer keyset")
    parser.add_argument("--tenant", required=True, help="tenant_id claim")
    parser.add_argument(
        "--role",
        required=True,
        help="role claim, e.g. export_worker, reviewer, admin",
    )
    parser.add_argument("--issuer", help="iss claim")
    parser.add_argument("--audience", help="aud claim")
    parser.add_argument(
        "--principal-ref",
        help="principal_ref claim identifying the acting operator; "
        "the server hashes it with the tenant to derive the stored principal",
    )
    parser.add_argument("--sub", help="sub claim, an alternative to --principal-ref")
    parser.add_argument(
        "--ttl-seconds",
        type=int,
        default=900,
        help="token lifetime in seconds (default 900)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    try:
        token = mint(
            key_path=args.key,
            kid=args.kid,
            tenant=args.tenant,
            role=args.role,
            issuer=args.issuer,
            audience=args.audience,
            ttl_seconds=args.ttl_seconds,
            principal_ref=args.principal_ref,
            subject=args.sub,
        )
    except MintError as error:
        print(f"mint-signed-token: {error}", file=sys.stderr)
        return 1
    print(token)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
