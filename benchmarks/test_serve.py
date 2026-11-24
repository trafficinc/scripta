#!/usr/bin/env python3
"""Check that measurement validation rejects corrupt or unexpected successes."""
import io
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import serve


class Connection:
    def __init__(self, status, body):
        self.response = io.BytesIO(body)
        self.response.status = status

    def request(self, *args, **kwargs):
        pass

    def getresponse(self):
        return self.response


class MeasurementTests(unittest.TestCase):
    def test_nearest_rank_percentiles_preserve_small_sample_boundaries(self):
        self.assertIsNone(serve.distribution([]))
        self.assertEqual(serve.distribution([7]),
                         {"count": 1, "min": 7, "max": 7, "p50": 7, "p95": 7, "p99": 7})
        result = serve.distribution([9, 1])
        self.assertEqual((result["p50"], result["p95"], result["p99"]), (1, 9, 9))

    def test_wrong_binary_json_and_stream_successes_are_rejected(self):
        for route, body in [("/upload", b"corrupt"), ("/download", b"corrupt"),
                            ("/json", b'{"ok":false}'), ("/stream", b"chunk-0\n")]:
            with self.subTest(route=route), self.assertRaises(RuntimeError):
                serve.exchange(Connection(200, body), route)
        self.assertEqual(serve.exchange(Connection(200, serve.PAYLOAD), "/upload")[0], 200)

    def test_overload_and_fault_statuses_do_not_silently_pass_normal_load(self):
        with self.assertRaises(RuntimeError):
            serve.exchange(Connection(503, b"unavailable"), "/json")
        self.assertEqual(serve.exchange(Connection(503, b"unavailable"), "/slow")[0], 503)
        self.assertEqual(serve.exchange(Connection(504, b"timeout"), "/json", soak=True)[0], 504)
        with self.assertRaises(RuntimeError):
            serve.exchange(Connection(502, b"bad gateway"), "/json", soak=True)

    def test_saturated_phase_separates_successes_from_fast_rejections(self):
        server = Mock(address="127.0.0.1:1")
        sampler = SimpleNamespace(phase=None)
        replies = [(200, 12.0, 10.0, 20), (503, 0.1, 0.1, 0)] * 4
        with patch.object(serve.http.client, "HTTPConnection"), patch.object(
                serve, "exchange", side_effect=[(200, 1.0, 1.0, 20)] * 2 + replies):
            result = serve.phase(server, sampler, "scaling", "/json", 1, 4, 8, soak=True)
        self.assertTrue(result["request_ceiling_reached"])
        self.assertEqual(result["statuses"], {"200": 4, "503": 4})
        self.assertEqual(result["success_latency_ms"]["count"], 4)
        self.assertEqual(result["success_latency_ms"]["min"], 12.0)
        self.assertEqual(result["successful_responses_per_second"],
                         result["responses_per_second"] / 2)
        self.assertEqual(server.recovered.call_count, 2)


if __name__ == "__main__":
    unittest.main()
