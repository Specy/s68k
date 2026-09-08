    move.l #6, d0           ; a = 6
    move.l #3, d1           ; b = 3
    move.l #2, d2           ; op = 2, the third entry of the table

    lea table, a0           ; the base of the table
    move.l d2, d3
    lsl.l #2, d3            ; op * 4, the size of a long
    move.l (a0, d3), a1     ; the address stored there
    jmp (a1)                ; and go to it

add_op:
    move.l d0, d4
    add.l d1, d4            ; a + b
    bra done
sub_op:
    move.l d0, d4
    sub.l d1, d4            ; a - b
    bra done
mul_op:
    move.l d0, d4
    mulu d1, d4             ; a * b
    bra done
div_op:
    move.l d0, d4
    divu d1, d4             ; a / b
    andi.l #$FFFF, d4       ; the quotient on its own
done:

    org $2000
table: dc.l add_op, sub_op, mul_op, div_op
