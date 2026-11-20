import subprocess,sys
for i in range(20): subprocess.run([sys.argv[1]],check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
print(20)
