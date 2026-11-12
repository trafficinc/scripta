import sys
s=0
for i in range(8):
 with open(sys.argv[1],encoding="utf-8") as f: s+=len(f.read())
print(s)
