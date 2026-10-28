* The field the instruction encodes the value in.
    addq.l #9,d0
    btst #40,d0
* An operand written as an address and stored as a distance from the
* instruction, and an address forced to a width that cannot name it.
    move.l far(pc),d0
    move.l $18000.w,d1
* A displacement `link` encodes in a word.
    link a6,#-70000
* How far a branch reaches: a byte when it is written `.s`, and a word for
* the rest, which is all a `dbra` has.
    bra.s far
    beq far
    dbra d0,far
    org $30000
far: dc.l 1
