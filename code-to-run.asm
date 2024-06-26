move.L #$1, D0   ; Load some values into registers
    move.L #$2, D1
    move.L #$3, A0
    bsr example_function 
    bra END 

example_function:
    movem.l a0, -(SP) ; Save registers to the stack
    move.l #$ff, D0
    move.l #$ff, D1
    move.l #$ff, A0
    movem.l (SP)+, a0 ; Restore registers from the stack
    rts     
END: