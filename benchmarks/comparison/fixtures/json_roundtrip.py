import json
data = {"name": "sample", "values": [1, 2, 3, 4], "meta": {"ok": True, "count": 7}}
total = 0
for i in range(20000):
    decoded = json.loads(json.dumps(data, separators=(",", ":")))
    if decoded != data:
        raise RuntimeError("roundtrip mismatch")
    total += decoded["meta"]["count"] + decoded["values"][3]
print(total)
