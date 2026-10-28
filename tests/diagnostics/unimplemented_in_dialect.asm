* The rest of the 68010 and the 68020 is still to come: each of their other
* instructions, and the 68020's shapes of a 68000 one, says so under its
* dialect, and a later instruction says so on the 68000 too.
    opt p=68020
    rtd #4
    muls.l d0,d1
    bfextu d0{4:8},d1
    extb.l d2
