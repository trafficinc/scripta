import json,sys,copy
with open(sys.argv[1]) as f: values=json.load(f)
copies={"seed":copy.deepcopy(values)}
for i in range(48): copies[str(i)]=copy.deepcopy(values)
print(len(copies))
