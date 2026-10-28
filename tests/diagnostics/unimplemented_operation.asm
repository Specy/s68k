* An instruction s68k leaves out for good, with the reason: the 68020's
* module call, which its 68020, a 68EC020, does not have.
    callm #0,(a0)
* A macro definition, and an invocation of it further down: the call site is a
* word the instruction table has never heard of, and what it needs is the
* feature, not the advice that it might be a label.
DELAY   macro
    move.b #23,d0
    endm

    DELAY 1
