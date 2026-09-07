SIZE    equ 40
LIMITX  equ 640-40
LIMITY  equ 480-40
BALL    equ $0000D2FF
BAR     equ $00808080
WHITE   equ $00FFFFFF

    move.b #92, d0
    move.b #17, d1
    trap #15                ; task 92 mode 17: draw off screen
    move.l #WHITE, d1
    move.b #80, d0
    trap #15                ; the pen, which outlines the ball

frame:
    move.b #11, d0
    move.w #$FF00, d1
    trap #15                ; clear the off screen image

    move.l #BALL, d1
    move.b #81, d0
    trap #15
    move.w ballx, d1        ; the box the ball is drawn inside
    move.w bally, d2
    move.w d1, d3
    add.w #SIZE, d3
    move.w d2, d4
    add.w #SIZE, d4
    move.b #88, d0
    trap #15                ; a filled ellipse in that box

    move.b #8, d0
    trap #15                ; task 8: hundredths of a second since the run started
    divu #640, d1
    swap d1
    andi.l #$FFFF, d1       ; the remainder, so the bar wraps at the right edge
    move.l d1, d3           ; where the bar ends
    move.l #BAR, d1
    move.b #81, d0
    trap #15
    move.l #0, d1
    move.l #0, d2
    move.l #8, d4
    move.b #87, d0
    trap #15                ; a bar as wide as the program has been running

    move.b #94, d0
    trap #15                ; the whole frame becomes visible here, at once

    move.b #23, d0
    move.l #2, d1
    trap #15                ; two hundredths of a second of program time

    move.w ballx, d5
    add.w stepx, d5
    cmp.w #0, d5
    blt flipx
    cmp.w #LIMITX, d5
    bgt flipx
    move.w d5, ballx
    bra movey
flipx:
    neg.w stepx             ; turn it round at the edge
movey:
    move.w bally, d5
    add.w stepy, d5
    cmp.w #0, d5
    blt flipy
    cmp.w #LIMITY, d5
    bgt flipy
    move.w d5, bally
    bra frame
flipy:
    neg.w stepy
    bra frame

ballx:  dc.w 100
bally:  dc.w 60
stepx:  dc.w 5
stepy:  dc.w 3
