* `extb.l` is the 68020's and `move ccr,<ea>` the 68010's. A 68000 program is
* told how the 68000 does the same, and offered the `opt p=` line that writes it
* for the later processor; the fixes are the two halves of the hint.
    org $1000
start:
    move.b #-3,d0
    extb.l d0
    move.w ccr,(a0)
    simhalt
