    lea numbers, a0     ; a0 points at the first number
    move.w #count-1, d1 ; dbra runs the loop one more time than the counter
    clr.l d0            ; sum = 0
loop:
    add.w (a0)+, d0     ; sum = sum + *a0, then step a0 to the next word
    dbra d1, loop       ; one number less to go

count equ 6
numbers: dc.w 4, 8, 15, 16, 23, 42
