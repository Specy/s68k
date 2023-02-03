ORG $1000
START:
    * Write here your code
    move.l #10000000, d0
for_start:
    sub.l #1, d0
    bpl for_start
END: * Jump here to end the program