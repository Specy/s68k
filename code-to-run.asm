ORG $1000
START:
    * Write here your code
limit equ 10000000
move.l #limit, d0
move.l #0, d1
for:
    add.l #1, d1
    sub.l #1, d0
    bne for
    
END: * Jump here to end the program