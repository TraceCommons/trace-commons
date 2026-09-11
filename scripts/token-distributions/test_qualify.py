import json
import io
import time
import unittest
from qualify import inspect_response, read_measured, ProcessSamples


class QualificationParser(unittest.TestCase):
    def frame(self, streaming=False):
        text = "é"
        tokens = [{"bytes": [value], "logprob": -1.25,
                   "top_logprobs": [{"bytes": [value], "logprob": -1.25}]}
                  for value in text.encode()]
        return {"id": "fixture", "choices": [{"index": 0, "finish_reason": "stop",
                             "delta" if streaming else "message": {"content": text},
                             "logprobs": {"content": tokens}}]}

    def test_json_and_sse_preserve_split_utf8_bytes(self):
        frame = self.frame()
        result = inspect_response(json.dumps(frame).encode(), False)
        self.assertEqual(result["tokens"], 2)
        self.assertNotIn("bytes", result)
        frame = self.frame(True)
        wire = ("data: " + json.dumps(frame) + "\n\ndata: [DONE]\n\n").encode()
        self.assertEqual(result, inspect_response(wire, True))
        with self.assertRaises(ValueError):
            inspect_response(wire.replace(b"data: [DONE]\n\n", b""), True)

    def test_stream_timing_reports_content_chunks_without_persisting_content(self):
        frame = self.frame(True)
        wire = ("data: " + json.dumps(frame) + "\n\ndata: [DONE]\n\n").encode()
        raw, timing = read_measured(io.BytesIO(wire), True, time.monotonic())
        self.assertEqual(raw, wire)
        self.assertEqual(timing["content_chunks"], 1)
        self.assertGreaterEqual(timing["ttft_ms"], 0)
        self.assertIsNone(timing["inter_content_chunk_mean_ms"])
        self.assertNotIn("é", json.dumps(timing))
        self.assertIsNone(ProcessSamples(None).metrics()["process_cpu_seconds"])

    def test_missing_or_mismatched_probabilities_are_not_support(self):
        for mutate in [lambda c: c.pop("logprobs"),
                       lambda c: c["message"].update(content="different"),
                       lambda c: c["logprobs"]["content"][0].update(logprob=float("nan"))]:
            frame = self.frame()
            mutate(frame["choices"][0])
            with self.assertRaises(ValueError):
                inspect_response(json.dumps(frame).encode(), False)


if __name__ == "__main__":
    unittest.main()
