text = ""
for i in range(100000):
    text = text + "abcdefgh"
print(f"{len(text)}:{int(text.replace('abcdefgh', '') == '')}")
