* The processor, and the 68020's bit fields, are chosen before the first
* instruction, which is judged and measured for the processor in force.
    org $1000
start:
    nop
    opt p=68020
    opt bit
    rts
