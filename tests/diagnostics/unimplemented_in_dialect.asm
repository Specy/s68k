* The rest of the 68020 is still to come: each of its other instructions,
* and its shapes of a 68000 one, says so under its dialect, and a later
* instruction says so on the 68000 too. The 68010 dialect is complete.
    opt p=68020
    cas.l d0,d1,(a0)
    muls.l d0,d1
    bfextu d0{4:8},d1
    extb.l d2
