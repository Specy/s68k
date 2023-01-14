amount equ 5000000
    * Write here your code
    move.l #amount, d0
for_start:
    add.l #1, d1
    sub.l #1, d0
    bpl for_start