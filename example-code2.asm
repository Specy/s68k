org $2000
test: dc.w 65535, 20, 30
START:
    move.w #1000, d0
    move.l #test, a0
    move.b (a0), d1