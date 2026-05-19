#!/usr/bin/env python3
"""
Pilot workload-token signer (operator-side helper).

Mints a short-lived EdDSA JWT that the client bearers to
`/v1/trace-upload-claim`. The issuer validates this JWT against the
configured workload public key, reads `invite_code` from the claims,
and checks the hash against the pilot allowlist before minting an
upload claim.

This is a pilot-shaped tool, not production. For real deployments,
mint workload tokens from a properly-managed issuer (KMS-backed key,
auditable mint API, etc.).

Usage:
  pip install --user pyjwt cryptography
  python3 sign-workload-token.py \\
      --private-key /etc/tracecommons/workload-signing-v1.pem \\
      --kid <workload-signing-kid> \\
      --invite-code <pilot-invite-code> \\
      --tenant-id tenant-<...> \\
      --principal-ref principal:<...> \\
      --issuer pilot-workload-issuer \\
      --audience trace-claim-issuer \\
      --ttl-seconds 3600

Prints the JWT to stdout. Set as the bearer for /v1/trace-upload-claim.
"""

import argparse
import datetime as dt
import sys

import jwt  # PyJWT


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument("--private-key", required=True, help="EdDSA private key PEM path")
    parser.add_argument("--kid", required=True, help="Key ID matching the issuer's expected workload key id")
    parser.add_argument("--invite-code", required=True, help="Pilot invite code (raw; the issuer hashes it)")
    parser.add_argument("--tenant-id", required=True)
    parser.add_argument("--principal-ref", required=True, help="Stable principal identifier the issuer attributes claims to")
    parser.add_argument("--issuer", required=True, help="iss claim — must match issuer's TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_ISSUER")
    parser.add_argument("--audience", required=True, help="aud claim — must match issuer's TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_AUDIENCE")
    parser.add_argument("--ttl-seconds", type=int, default=3600)
    parser.add_argument(
        "--consent-scope",
        action="append",
        default=["debugging_evaluation"],
        help="Repeatable; defaults to ['debugging_evaluation']",
    )
    parser.add_argument(
        "--allowed-use",
        action="append",
        default=["debugging"],
        help="Repeatable; defaults to ['debugging']",
    )
    args = parser.parse_args()

    with open(args.private_key, "rb") as f:
        private_key_pem = f.read()

    now = dt.datetime.now(dt.timezone.utc)
    claims = {
        "iss": args.issuer,
        "aud": args.audience,
        "sub": args.principal_ref,
        "principal_ref": args.principal_ref,
        "tenant_id": args.tenant_id,
        "iat": int(now.timestamp()),
        "exp": int((now + dt.timedelta(seconds=args.ttl_seconds)).timestamp()),
        "allowed_consent_scopes": args.consent_scope,
        "allowed_uses": args.allowed_use,
        "invite_code": args.invite_code,
    }
    token = jwt.encode(
        claims,
        private_key_pem,
        algorithm="EdDSA",
        headers={"kid": args.kid},
    )
    print(token)
    return 0


if __name__ == "__main__":
    sys.exit(main())
