    move.w #365, d0     ; days = 365
    move.w #24, d1
    mulu d1, d0         ; hours = days * 24

    move.l #1000, d2    ; seconds = 1000
    divu #60, d2        ; d2 = seconds / 60, with seconds % 60 above it
    move.l d2, d3
    andi.l #$FFFF, d2   ; the quotient, whole minutes
    swap d3
    andi.l #$FFFF, d3   ; the remainder, the seconds left over
