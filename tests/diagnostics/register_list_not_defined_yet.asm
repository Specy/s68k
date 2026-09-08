* A register list has to be defined above the `movem` that reads it: EASy68K's
* "Register list symbol not previously defined".
start:
    movem.l AllRegs,-(a7)
AllRegs reg d0-d2/a0
