* A static bit number past the destination's width assembles, as in EASy68K,
* and the 68000 takes it modulo the width: 8 bits in memory, 32 in Dn.
    btst #9,(a0)
    bset #33,d0
    bclr #-1,d1
* Within the width there is nothing to say.
    btst #7,(a0)
    bchg #31,d2
    rts
