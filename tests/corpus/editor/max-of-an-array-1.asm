count equ 8

    lea numbers, a0     ; a0 points at the first element
    move.w (a0)+, d0    ; best = numbers[0]
    clr.w d3            ; where = 0
    clr.w d4            ; i = 0
    move.w #count-2, d1 ; seven elements left, and dbra counts one more
loop:
    addq.w #1, d4       ; i++
    move.w (a0)+, d2    ; n = *a0++
    cmp.w d0, d2        ; n - best
    ble not_bigger      ; if(n <= best) keep the one we have
    move.w d2, d0       ; best = n
    move.w d4, d3       ; where = i
not_bigger:
    dbra d1, loop

    org $2000
numbers: dc.w 12, -4, 37, 8, 99, 41, 2, 60
