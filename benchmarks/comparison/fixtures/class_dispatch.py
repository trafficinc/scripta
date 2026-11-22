class Counter:
    def __init__(self):
        self.value = 0
    def next(self):
        self.value += 1
        return self.value
counter = Counter()
total = 0
for i in range(200000):
    total += counter.next()
print(f"{counter.value}:{total}")
