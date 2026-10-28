* Every rejection names what was found and what is allowed there.
    clr a0
    divu a0,d0
    jmp (a0)+
    move.l d0,greeting(pc)
greeting: dc.b 'hello',0
    addi a0,d0
* Two operands each allowed where it stands, which the 68000 has no
* encoding for together.
    add.l a0,(a1)
    btst #1,#5
