ROWS equ 3
COLS equ 4

    lea grid, a0
    move.l #2, d0           ; row = 2
    move.l #1, d1           ; col = 1
    move.l d0, d2
    mulu #COLS, d2          ; row * COLS
    add.l d1, d2            ; + col
    add.l d2, d2            ; times 2, the size of a word
    move.w (a0, d2), d3     ; d3 = grid[row][col]

    lea grid, a1
    move.l d1, d4
    add.l d4, d4            ; col * 2
    add.l d4, a1            ; a1 = &grid[0][col]
    clr.w d5                ; total = 0
    move.w #ROWS-1, d6
column:
    add.w (a1), d5          ; total += grid[r][col]
    add.l #COLS*2, a1       ; down one row, a whole row of bytes
    dbra d6, column

    org $2000
grid:   dc.w 1, 2, 3, 4
        dc.w 10, 20, 30, 40
        dc.w 100, 200, 300, 400
