* A real 68000 instruction s68k does not assemble, with the reason.
    rte
    reset
    trap #3
* A macro definition, and an invocation of it further down: the call site is a
* word the instruction table has never heard of, and what it needs is the
* feature, not the advice that it might be a label.
DELAY   macro
    move.b #23,d0
    endm

    DELAY 1
