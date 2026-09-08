* The number of operands an instruction takes.
    move.l d0
    rts d0
* A data directive takes a list, so its count is a minimum, and its message
* names the size the same way `value_expected` does.
    dc.b
    ds.w
    dcb.l 4
