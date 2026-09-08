SKY     equ $00E0B070       ; a colour is $00BBGGRR: blue, green, then red
GRASS   equ $003C9648
SUN     equ $0000D2FF
WALL    equ $004070C0
ROOF    equ $002020A0
DOOR    equ $00204070
WHITE   equ $00FFFFFF

    move.l #SKY, d1
    bsr both_colours
    move.l #0, d1           ; left
    move.l #0, d2           ; top
    move.l #640, d3         ; right
    move.l #320, d4         ; bottom
    move.b #87, d0          ; task 87: a filled rectangle, the sky
    trap #15

    move.l #GRASS, d1
    bsr both_colours
    move.l #0, d1
    move.l #320, d2
    move.l #640, d3
    move.l #480, d4
    move.b #87, d0          ; the ground
    trap #15

    move.l #SUN, d1
    bsr both_colours
    move.l #500, d1
    move.l #40, d2
    move.l #600, d3
    move.l #140, d4
    move.b #88, d0          ; task 88: a filled ellipse in a square box, the sun
    trap #15

    move.l #WALL, d1
    move.b #81, d0          ; task 81: the fill colour on its own
    trap #15
    move.l #WHITE, d1
    move.b #80, d0          ; task 80: a different pen, so the walls get an outline
    trap #15
    move.b #3, d1
    move.b #93, d0          ; task 93: the pen width, three pixels
    trap #15
    move.l #200, d1
    move.l #200, d2
    move.l #440, d3
    move.l #380, d4
    move.b #87, d0          ; the house
    trap #15

    move.l #ROOF, d1
    bsr both_colours
    move.l #180, d1
    move.l #200, d2
    move.b #86, d0          ; task 86: move the drawing point, drawing nothing
    trap #15
    move.l #320, d1
    move.l #110, d2
    move.b #85, d0          ; task 85: a line from the drawing point to here
    trap #15
    move.l #460, d1
    move.l #200, d2
    move.b #85, d0          ; and on to the other eave
    trap #15
    move.l #180, d1
    move.l #200, d2
    move.b #85, d0          ; and back where it started
    trap #15
    move.l #320, d1
    move.l #170, d2
    move.b #89, d0          ; task 89: a flood fill out from a point inside
    trap #15

    move.l #DOOR, d1
    bsr both_colours
    move.l #290, d1
    move.l #290, d2
    move.l #350, d3
    move.l #380, d4
    move.b #87, d0          ; the door
    trap #15

    move.l #WHITE, d1
    move.b #80, d0
    trap #15
    lea label, a1
    move.l #210, d1
    move.l #420, d2
    move.b #95, d0          ; task 95: text at a pixel position
    trap #15

    move.b #9, d0
    trap #15

* both_colours(c): the fill and the pen both become the colour in d1
both_colours:
    move.b #81, d0
    trap #15
    move.b #80, d0
    trap #15
    rts

    org $3000
label: dc.b 'Seven shapes and a line of text', 0
