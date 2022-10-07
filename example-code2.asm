org $2000
test: dc.w 128, 20, 30
START:
    move.w #1000, d0
    move.w #test, a0
    move.w (a0), d1
    add #100, d1
