* `addx`, `subx`, `abcd` and `sbcd` take two data registers or two predecrement
* operands. Neither operand of the first line is one of the four, and each of
* the next two is half of a shape the other half does not finish.
    addx.l #1,d0
    addx.l d0,-(a1)
    sbcd -(a0),d1
* Both shapes, written properly, assemble and say nothing.
    addx.l d0,d1
    addx.l -(a0),-(a1)
