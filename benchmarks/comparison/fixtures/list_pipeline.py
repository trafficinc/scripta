values = list(range(2000))
total = 0
count = 0
for batch in range(200):
    mapped = [x * 3 for x in values]
    filtered = [x for x in mapped if x % 2 == 0]
    count += len(filtered)
    for x in filtered:
        total += x
print(f"{count}:{total}")
