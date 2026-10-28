* A short branch to the instruction right after it has a displacement of 0,
* the byte a 68000 reads as the mark of a word branch.
    bra.s next
next:
    beq.s done
done:
* With no size the assembler gives it a word, and it is fine.
    bne done2
done2:
    rts
