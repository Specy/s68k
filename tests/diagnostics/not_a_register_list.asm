* A name where `movem` reads a register list has to be one: EASy68K's "Symbol
* is not a register list symbol".
count   equ 4
start:
    movem.l count,-(a7)
