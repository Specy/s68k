START:
amount equ 1000000
    move.l #amount, d0
for_start:
    add.l #1, d1
    sub.l #1, d0
    bpl for_start