items = {}
for i in range(256):
    items["key" + str(i)] = i
total = 0
for i in range(40960):
    key = "key" + str(i % 256)
    items[key] += 1
    total += items[key]
for i in range(128):
    updated = items.copy()
    del updated["key" + str(i)]
    items = updated
print(f"{len(items)}:{total}:{sum(items.values())}")
