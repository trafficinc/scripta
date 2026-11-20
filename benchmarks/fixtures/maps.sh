declare -A values; for ((i=0;i<128;i++)); do values[$i]=$i; done; sum=0; for ((i=0;i<8000;i++)); do ((sum+=values[64])); done; printf "%s\n" "$sum"
