import csv
import json
import sys
from pathlib import Path

directory = Path(sys.argv[1])
with (directory / "input.csv").open(encoding="utf-8", newline="") as source:
    rows = list(csv.reader(source))
values = [{"id": int(row[0]), "name": row[1], "value": int(row[2]) * 3} for row in rows]
target = directory / "output.json"
target.write_bytes(json.dumps(values, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))
decoded = json.loads(target.read_bytes())
if decoded != values:
    raise ValueError("JSON output mismatch")
total = 0
for row in decoded:
    total += row["id"] + row["value"]
print(f"{len(decoded)}:{total}")
