    move.l #182, d0     ; n = 182, which is %10110110

    btst #0, d0         ; is the lowest bit set?
    sne d1              ; d1 = $FF when n is odd, $00 when it is even

    move.l d0, d2
    lsl.l #3, d2        ; n * 8, three places left is eight times

    move.l d0, d3
    andi.l #$0F, d3     ; the low nibble on its own

    clr.l d4            ; bits = 0
    move.l d0, d5       ; a copy to take apart
    move.w #31, d6      ; 32 bits, so 31
count:
    lsr.l #1, d5        ; the lowest bit falls into C
    bcc no_bit
    addq.l #1, d4       ; bits++
no_bit:
    dbra d6, count
