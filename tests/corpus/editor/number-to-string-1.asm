    move.l #48879, d0       ; n = 48879
    move.l #16, d1          ; in hexadecimal
    bsr print_in_base
    move.l #48879, d0
    move.l #10, d1          ; in decimal
    bsr print_in_base
    move.l #48879, d0
    move.l #2, d1           ; in binary
    bsr print_in_base
    move.b #9, d0
    trap #15

* print_in_base(n, base): n in d0, base in d1
print_in_base:
    lea buffer_end, a1      ; build the text backwards from the end
    clr.b -(a1)             ; the terminator goes down first
digit:
    move.l d0, d2
    divu d1, d2             ; d2 = n / base, with n % base above it
    move.l d2, d3
    swap d3
    andi.l #$FFFF, d3       ; the digit
    andi.l #$FFFF, d2       ; n = n / base
    cmp.b #9, d3
    bhi letter
    add.b #'0', d3          ; 0 to 9 become '0' to '9'
    bra store
letter:
    add.b #'A'-10, d3       ; 10 and up become 'A' and up
store:
    move.b d3, -(a1)        ; in front of the digits we have already
    move.l d2, d0           ; n = n / base, and the move sets Z
    bne digit               ; until nothing is left of it
    move.b #13, d0          ; task 13: print the string at a1 and a new line
    trap #15
    rts

    org $2000
buffer:     ds.b 34         ; 32 binary digits, the terminator and a spare byte
buffer_end:
