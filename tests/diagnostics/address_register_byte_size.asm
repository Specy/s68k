* An address register is never used one byte at a time, in either direction:
* the message holds whether the register is the operand written...
    move.b d0,a0
* ...or the one read.
    cmp.b a0,d1
