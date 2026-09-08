* An address is forced to `.w` or `.l`, and to nothing else: the size the
* instruction works at goes after the mnemonic.
    move.l table.b,d0
    bra done.s
* The same two lines written the way they are meant, and the `.w` form, which
* is fine and is range checked against the sixteen bits it claims.
    move.b table,d0
    move.l table.w,d1
    bra.s done
table: dc.l 1
done: rts
