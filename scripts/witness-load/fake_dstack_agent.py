#!/usr/bin/env python3
"""A stand-in dstack guest agent, for running the witness on a laptop.

LOCAL LOAD TESTING ONLY. A witness booted against this agent signs with a
fixed, published test key and reports a measurement spliced into a borrowed
fixture quote. Nothing anywhere accepts that measurement, which is the point:
a certificate it issues is worthless outside this harness.

It serves the two calls the witness makes, over a unix socket, in the shape
`DstackSocketAgent` reads (crates/trace-commons-server/src/witness_service/
enclave.rs):

  GET /v0/GetKey?path=..&purpose=..&algorithm=..  -> {"key": hex, "signature_chain": [hex, hex]}
  GET /v0/GetQuote?report_data=0x<hex>             -> {"quote": hex}

The quote is the real TDX quote from the attestation crate's NEAR AI fixture,
with `report_data` spliced in at the offset the witness parses, so the
witness's own quote parser runs unchanged.

Usage: fake_dstack_agent.py --socket /path/to/agent.sock
"""

import argparse
import json
import os
import socketserver
import sys
import urllib.parse

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
FIXTURE = os.path.join(
    REPO,
    "crates/trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json",
)

# quote_offsets::REPORT_DATA in enclave.rs: the 48-byte quote header, then 520
# bytes into the TD report body.
REPORT_DATA_OFFSET = 48 + 520
REPORT_DATA_LEN = 64

# secp256k1 scalar 2: a published test value, never a secret.
TEST_SCALAR = (2).to_bytes(32, "big").hex()


def load_quote():
    with open(FIXTURE, "r", encoding="utf-8") as handle:
        report = json.load(handle)
    quote = report["intel_quote"]
    if quote.startswith("0x"):
        quote = quote[2:]
    return bytearray.fromhex(quote)


QUOTE = load_quote()


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        request_line = self.rfile.readline().decode("latin-1").strip()
        while True:
            line = self.rfile.readline()
            if not line or line in (b"\r\n", b"\n"):
                break
        parts = request_line.split(" ")
        if len(parts) < 2:
            return self.reply(400, {"error": "bad request"})
        url = urllib.parse.urlparse(parts[1])
        query = urllib.parse.parse_qs(url.query)
        if url.path in ("/v0/GetKey", "/GetKey"):
            return self.reply(
                200, {"key": TEST_SCALAR, "signature_chain": ["01" * 65, "02" * 65]}
            )
        if url.path in ("/v0/GetQuote", "/GetQuote"):
            report_data = query.get("report_data", [""])[0]
            if report_data.startswith("0x"):
                report_data = report_data[2:]
            data = bytes.fromhex(report_data)
            if len(data) > REPORT_DATA_LEN:
                return self.reply(400, {"error": "report data too long"})
            data = data.ljust(REPORT_DATA_LEN, b"\0")
            quote = bytearray(QUOTE)
            quote[REPORT_DATA_OFFSET : REPORT_DATA_OFFSET + REPORT_DATA_LEN] = data
            return self.reply(200, {"quote": quote.hex()})
        return self.reply(404, {"error": "not found"})

    def reply(self, status, body):
        payload = json.dumps(body).encode("utf-8")
        head = (
            f"HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n"
            f"Content-Length: {len(payload)}\r\nConnection: close\r\n\r\n"
        )
        self.wfile.write(head.encode("latin-1") + payload)


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--socket", required=True)
    args = parser.parse_args()
    if os.path.exists(args.socket):
        os.unlink(args.socket)
    with Server(args.socket, Handler) as server:
        print(f"fake dstack agent on {args.socket}", file=sys.stderr, flush=True)
        try:
            server.serve_forever()
        finally:
            os.unlink(args.socket)


if __name__ == "__main__":
    main()
