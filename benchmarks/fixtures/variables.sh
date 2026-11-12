x=1; sum=0; for ((i=0;i<20000;i++)); do ((sum+=x)); done; printf "%s\n" "$sum"
