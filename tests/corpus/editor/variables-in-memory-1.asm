TAX equ 20

    move.l price, d0        ; total = price
    add.l shipping, d0      ; total = total + shipping
    add.l #TAX, d0          ; total = total + TAX
    move.l d0, total        ; write the answer back into memory

    org $2000
price:    dc.l 250
shipping: dc.l 35
total:    ds.l 1
