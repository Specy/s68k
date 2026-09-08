    move.l #84, d0      ; a = 84
    move.l #36, d1      ; b = 36
    bsr gcd             ; a = gcd(a, b)
    move.l d0, d2       ; the answer, kept somewhere it will not be reused
    bra end

* gcd(a, b): a arrives in d0 and b in d1, the answer leaves in d0.
* It works in d3, which the caller has to expect.
gcd:
    tst.l d1            ; while(b != 0)
    beq gcd_done
    move.l d0, d3       ; t = a
    divu d1, d3         ; d3 = a / b, with a % b above it
    swap d3
    andi.l #$FFFF, d3   ; t = a % b
    move.l d1, d0       ; a = b
    move.l d3, d1       ; b = t
    bra gcd
gcd_done:
    rts

end:
