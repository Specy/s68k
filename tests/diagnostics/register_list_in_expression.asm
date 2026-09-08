* A register list stands for a `movem` operand and has no value, so it cannot
* be part of an expression: EASy68K's "Register list symbol used in an
* expression".
AllRegs reg d0-d7/a0-a6
start:
    move.l #AllRegs,d0
    move.l AllRegs,d1
    move.l AllRegs+1,d2
