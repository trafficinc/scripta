import sys
from pathlib import Path

directory = Path(sys.argv[1])
text, count = "", 0
with (directory / "input.txt").open(encoding="utf-8", newline="") as source:
    for line in source:
        text += line.rstrip("\r\n").upper() + "\n"
        count += 1
target = directory / "output.txt"
target.write_bytes(text.encode("utf-8"))
actual = target.read_bytes()
if actual != text.encode("utf-8"):
    raise ValueError("file output mismatch")
print(f"{count}:{len(actual)}")
