* A written number wider than the machine: EASy68K's "Numeric constant exceeds
* 32 bits". The value is kept whole and checked against the size it is used at.
big equ $1234567890
    move.l #big,d0
    move.l #4294967296,d1
* `$ffffffff` is the widest that fits, and says nothing.
    move.l #$ffffffff,d2
