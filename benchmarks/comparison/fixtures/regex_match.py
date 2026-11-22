import re
texts = ["user-123@example.com", "invalid address", "a-9@host.org", "missing@"]
total = 0
for i in range(20000):
    if re.search(r"^[a-z]+-[0-9]+@[a-z]+[.](com|org)$", texts[i % 4]) is not None:
        total += 1
print(total)
