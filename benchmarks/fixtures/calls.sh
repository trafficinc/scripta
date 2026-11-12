add() { result=$(($1+1)); }; sum=0; for ((i=0;i<10000;i++)); do add "$sum"; sum=$result; done; printf "%s\n" "$sum"
