def increment(value):
    return value + 1
total = 0
for i in range(200000):
    total += increment(i)
print(total)
