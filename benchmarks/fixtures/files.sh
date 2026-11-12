size=0; for ((i=0;i<8;i++)); do text=$(cat -- "$1"); ((size+=${#text})); done; printf "%s\n" "$size"
