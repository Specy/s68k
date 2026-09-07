    move.l #30, d0      ; width = 30
    move.l #12, d1      ; height = 12
    move.l d0, d2       ; perimeter = width
    add.l d1, d2        ; perimeter = perimeter + height
    add.l d2, d2        ; perimeter = perimeter + perimeter

    move.l #$FFFFFF00, d3   ; d3 already holds something
    move.b d2, d3           ; only the lowest byte of d3 is written
