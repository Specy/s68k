count equ 10

    lea numbers, a0     ; a0 points at the first element
    move.w #count-1, d1 ; dbra runs the loop one more time than the counter
    move.w #1, d0       ; n = 1
fill:
    move.w d0, (a0)     ; *a0 = n
    addq.l #2, a0       ; step a0 on to the next word
    addq.w #1, d0       ; n++
    dbra d1, fill

    org $2000
numbers: ds.w count
