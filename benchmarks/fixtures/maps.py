values={str(i):i for i in range(128)}
s=0
for i in range(8000): s+=values["64"]
print(s)
