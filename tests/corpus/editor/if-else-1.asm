    move.l #37, d0      ; a = 37
    move.l #64, d1      ; b = 64

    cmp.l d1, d0        ; a - b
    bge a_is_bigger     ; if(a >= b) goto a_is_bigger
    move.l d1, d2       ; bigger = b
    bra done
a_is_bigger:
    move.l d0, d2       ; bigger = a
done:

    move.l d0, d3       ; distance = a
    sub.l d1, d3        ; distance = distance - b
    bpl positive        ; if(distance >= 0) it is already the answer
    neg.l d3            ; otherwise flip its sign
positive:
