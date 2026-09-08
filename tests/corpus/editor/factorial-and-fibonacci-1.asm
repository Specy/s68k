    move.l #8, -(sp)        ; n = 8
    bsr factorial
    add.l #4, sp
    move.l d0, d6           ; 8!
    move.l #10, -(sp)       ; n = 10
    bsr fib
    add.l #4, sp
    move.l d0, d7           ; fib(10)
    bra end

* factorial(n): n at 8(a6), the answer in d0
factorial:
    link a6, #0             ; a frame with no locals, only the argument
    move.l 8(a6), d0        ; n
    cmp.l #1, d0
    ble one                 ; if(n <= 1) return 1
    subq.l #1, d0
    move.l d0, -(sp)
    bsr factorial           ; factorial(n - 1)
    add.l #4, sp
    move.l 8(a6), d1        ; n again, out of this call's own frame
    mulu d1, d0             ; n * factorial(n - 1)
    bra factorial_done
one:
    move.l #1, d0
factorial_done:
    unlk a6
    rts

* fib(n): n at 8(a6), one long of local room at -4(a6), the answer in d0
fib:
    link a6, #-4
    move.l 8(a6), d0        ; n
    cmp.l #2, d0
    blt fib_done            ; fib(0) is 0 and fib(1) is 1
    subq.l #1, d0
    move.l d0, -(sp)
    bsr fib                 ; fib(n - 1)
    add.l #4, sp
    move.l d0, -4(a6)       ; kept across the second call
    move.l 8(a6), d0
    subq.l #2, d0
    move.l d0, -(sp)
    bsr fib                 ; fib(n - 2)
    add.l #4, sp
    add.l -4(a6), d0        ; fib(n - 1) + fib(n - 2)
fib_done:
    unlk a6
    rts

end:
