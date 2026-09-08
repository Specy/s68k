    lea greeting, a1    ; the address of the string
    move.b #13, d0      ; task 13: print it and go to a new line
    trap #15

    lea question, a1
    move.l #42, d1      ; the number
    move.b #17, d0      ; task 17: print the string, then the number
    trap #15

    move.b #9, d0       ; task 9: end the program
    trap #15

    org $2000
greeting: dc.b 'Hello, world!', 0
question: dc.b 'The answer is ', 0
