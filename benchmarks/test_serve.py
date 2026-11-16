#!/usr/bin/env python3
"""Check that measurement validation rejects corrupt or unexpected successes."""
import io
import unittest

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


if __name__ == "__main__":
    unittest.main()
