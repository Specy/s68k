    move.l #4, -(sp)        ; the second argument, b
    move.l #3, -(sp)        ; the first argument, a
    bsr sum_of_squares
    add.l #8, sp            ; the caller takes the two arguments back off
    move.l d0, d7           ; the answer
    bra end

* sum_of_squares(a, b): a at 8(a6), b at 12(a6), the answer leaves in d0
sum_of_squares:
    link a6, #-4            ; a frame with four bytes of local room
    move.l 8(a6), d0        ; a
    bsr square
    move.l d0, -4(a6)       ; local = a * a, kept across the next call
    move.l 12(a6), d0       ; b
    bsr square
    add.l -4(a6), d0        ; a * a + b * b
    unlk a6
    rts

* square(x): x in d0, the answer in d0, and d1 is given back as it was found
square:
    move.l d1, -(sp)        ; the caller's d1, saved
    move.l d0, d1
    mulu d1, d0             ; x * x
    move.l (sp)+, d1        ; and given back
    rts

end:
