sum=0; for ((i=0;i<150;i++)); do for ((j=0;j<150;j++)); do ((sum+=1)); done; done; printf "%s\n" "$sum"
