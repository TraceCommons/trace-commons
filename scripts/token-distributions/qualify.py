#!/usr/bin/env python3
"""Synthetic Chat probability probe. Writes metadata only; never response text.

A successful probe establishes wire support for this exact model and endpoint
at this time, not probability semantics, attestation, or production readiness.
"""
import argparse
import datetime
import hashlib
import ipaddress
import json
import math
import os
import time
import urllib.error
import urllib.parse
import urllib.request

LIMIT = 8 * 1024 * 1024


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("redirect-refused")


def inspect_response(raw, streaming):
    if streaming:
        frames = []
        done = False
        for line in raw.decode("utf-8").splitlines():
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                done = True
            elif data:
                if done:
                    raise ValueError("data-after-done")
                frames.append(json.loads(data))
        if not done:
            raise ValueError("incomplete-stream")
    else:
        frames = [json.loads(raw)]
    text, records, finished = [], [], False
    response_id = None
    for frame in frames:
        current_id = frame.get("id")
        if not isinstance(current_id, str) or not current_id or len(current_id) > 256:
            raise ValueError("response-id-unavailable")
        if response_id is not None and response_id != current_id:
            raise ValueError("response-id-changed")
        response_id = current_id
        if len(frame.get("choices", [])) > 1:
            raise ValueError("unsupported-choice")
        for choice in frame.get("choices", []):
            if choice.get("index") != 0:
                raise ValueError("unsupported-choice")
            message = choice.get("delta" if streaming else "message", {})
            if message.get("tool_calls") or message.get("reasoning_content"):
                raise ValueError("unsupported-content")
            text.append(message.get("content") or "")
            records.extend((choice.get("logprobs") or {}).get("content") or [])
            finished |= choice.get("finish_reason") == "stop"
    if not finished or not records:
        raise ValueError("probabilities-unavailable")
    chosen = bytearray()
    alternative_counts = []
    for record in records:
        alternatives = record.get("top_logprobs") or []
        alternative_counts.append(len(alternatives))
        for token in [record] + alternatives:
            raw_bytes = token.get("bytes")
            probability = token.get("logprob")
            if not isinstance(raw_bytes, list) or not raw_bytes:
                raise ValueError("token-bytes-unavailable")
            if any(type(b) is not int or not 0 <= b <= 255 for b in raw_bytes):
                raise ValueError("invalid-token-bytes")
            if type(probability) not in (float, int) or not math.isfinite(probability) or probability > 0:
                raise ValueError("probability-semantics-unsupported")
        chosen.extend(record["bytes"])
    if bytes(chosen) != "".join(text).encode("utf-8"):
        raise ValueError("byte-reconstruction-mismatch")
    return {"tokens": len(records), "minimum_alternatives": min(alternative_counts),
            "maximum_alternatives": max(alternative_counts), "exact_bytes": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True, help="Exact Chat Completions HTTPS endpoint")
    parser.add_argument("--model", required=True)
    parser.add_argument("--backend", required=True)
    parser.add_argument("--token-env", default="TOKEN_PROBE_API_KEY")
    parser.add_argument("--top-k", type=int, choices=range(21), default=20)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    endpoint = urllib.parse.urlsplit(args.endpoint)
    try:
        local = ipaddress.ip_address(endpoint.hostname or "").is_loopback
    except ValueError:
        local = False
    if endpoint.username or endpoint.password or endpoint.query or endpoint.fragment or not (
        endpoint.scheme == "https" or (endpoint.scheme == "http" and local)
    ):
        parser.error("Use HTTPS or a literal loopback address without URL credentials or query parameters")
    token = os.environ.get(args.token_env)
    if not token:
        parser.error("The named token environment variable is unset")
    opener = urllib.request.build_opener(NoRedirect, urllib.request.ProxyHandler({}))
    result = {"version": 1, "backend": args.backend, "model": args.model,
              "at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "endpoint_digest": hashlib.sha256(args.endpoint.encode()).hexdigest(),
              "semantics": "unknown", "conditioning": "unknown", "probes": []}
    for probabilities, streaming in [(False, False), (True, False), (True, True)]:
        body = {"model": args.model, "messages": [{"role": "user", "content": "Reply exactly: blue sky."}],
                "max_tokens": 32, "stream": streaming}
        if probabilities:
            body.update(logprobs=True, top_logprobs=args.top_k)
        probe = {"requested_probabilities": probabilities, "streaming": streaming,
                 "requested_top_k": args.top_k if probabilities else None}
        started = time.monotonic()
        try:
            request = urllib.request.Request(args.endpoint, data=json.dumps(body).encode(),
                headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
            with opener.open(request, timeout=60) as response:
                raw = response.read(LIMIT + 1)
                probe["http_status"] = response.status
            if len(raw) > LIMIT:
                raise ValueError("response-too-large")
            probe["response_bytes"] = len(raw)
            if probabilities:
                probe.update(inspect_response(raw, streaming))
            probe["status"] = "supported" if probabilities else "baseline"
        except urllib.error.HTTPError as error:
            probe.update(status="unavailable", http_status=error.code)
        except (ValueError, KeyError, TypeError, OSError):
            probe["status"] = "unavailable"
        probe["elapsed_ms"] = round(1000 * (time.monotonic() - started), 2)
        result["probes"].append(probe)
    # Never persist token bytes, probabilities, credentials, or provider errors.
    with open(args.output, "x", encoding="utf-8") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    return 0 if all(p["status"] != "unavailable" for p in result["probes"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
