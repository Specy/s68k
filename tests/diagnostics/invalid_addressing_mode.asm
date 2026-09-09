* Every rejection names what was found and what is allowed there.
    clr a0
    divu a0,d0
    jmp (a0)+
    move.l d0,greeting(pc)
greeting: dc.b 'hello',0
    addi a0,d0
