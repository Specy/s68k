    lea text, a0        ; p = text
    move.l a0, d1       ; keep where the string starts
scan:
    tst.b (a0)+         ; is *p++ the terminator?
    bne scan
    move.l a0, d0       ; p, one byte past the terminator
    sub.l d1, d0        ; n = p - text
    subq.l #1, d0       ; without the terminator itself

    org $2000
text: dc.b 'Assembly is fun', 0
