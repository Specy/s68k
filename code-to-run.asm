    lea test, a1
    move.l #13, d0
    trap #15
test: dc.b 'test', 0