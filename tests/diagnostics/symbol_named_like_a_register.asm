* `za0` to `za7`, `zd0` to `zd7` and `zpc` are the 68020's suppressed
* registers only where no symbol has the name: a symbol keeps its meaning
* wherever it can stand, which a 68020 program is told.
    opt p=68020
    lea (za0),a1
    lea (zpc,a0),a2
* The second place of the parentheses holds no symbol, so `za0` is the
* register there, and nothing is said.
    lea (4,za0,d1),a3
    rts
za0: dc.w 0
zpc equ 8
