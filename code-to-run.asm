    move.l #test, a0
    jsr (a0)
    bra end

ORG $2000
test: 
    move.l #-1, d0
    move.l (sp), a3
    rts
end: