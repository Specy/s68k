count equ 12

    lea numbers, a0     ; the array
    move.l #91, d0      ; the value we are looking for
    clr.l d1            ; low = 0
    move.l #count-1, d2 ; high = count - 1
    moveq #-1, d3       ; found = -1, meaning not there
search:
    cmp.l d2, d1        ; low - high
    bgt search_done     ; while(low <= high)
    move.l d1, d4
    add.l d2, d4
    lsr.l #1, d4        ; mid = (low + high) / 2
    move.l d4, d5
    add.l d5, d5        ; mid * 2, the size of a word
    move.w (a0, d5), d6 ; numbers[mid]
    cmp.w d0, d6        ; numbers[mid] - target
    beq found
    bgt too_big
    move.l d4, d1
    addq.l #1, d1       ; low = mid + 1
    bra search
too_big:
    move.l d4, d2
    subq.l #1, d2       ; high = mid - 1
    bra search
found:
    move.l d4, d3       ; found = mid
search_done:

    org $2000
numbers: dc.w 2, 5, 8, 12, 16, 23, 38, 56, 72, 91, 100, 127
