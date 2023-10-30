limit equ 10000000
move.l #limit, d0
move.l #0, d1
for:
    add.l #1, d1
    sub.l #1, d0
    bne for