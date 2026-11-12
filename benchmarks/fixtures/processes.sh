for ((i=0;i<20;i++)); do "$1" || exit 1; done; printf "20\n"
