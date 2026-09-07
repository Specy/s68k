    lea first, a1
    move.b #18, d0      ; task 18: print the prompt, then read a number
    trap #15
    move.l d1, d2       ; a = what was typed

    lea second, a1
    move.b #18, d0
    trap #15
    add.l d1, d2        ; a = a + b

    lea answer, a1
    move.l d2, d1       ; task 17 prints the number in d1
    move.b #17, d0      ; task 17: the string, then the number
    trap #15

    move.b #9, d0
    trap #15

    org $2000
first:  dc.b 'First number: ', 0
second: dc.b 10, 'Second number: ', 0
answer: dc.b 10, 'The sum is ', 0
