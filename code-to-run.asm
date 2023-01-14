ORG $1000
START:
    * Write here your code
    move.l #-1, d0
    bsr p1
    add.l #1, d3

    bra END
p1:
    add.l #1, d0
    bsr p2
    add.l #1, d1
    rts
p2:
    add.l #1, d2
    rts




END: * Jump here to end the program