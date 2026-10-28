* On the 68020 a short branch cannot reach one byte past itself: its
* displacement, -1, is the byte $FF, which marks a long branch there.
    opt p=68020
    org $1000
start:
    bra.s *+1
* With no size the assembler gives it a word, and it is fine.
    bra *+1
    rts
