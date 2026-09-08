* A space where the comma goes: the operand field ends at it, so `d1` is
* EASy68K's bare comment field and the line really does have one operand.
    move.l  d0 d1
    add.w   d0 d1
* The rule is narrow: a comment that happens to open with a register's name
* says nothing, and neither does a line whose operands are all there.
    move.l  d0 d1 is where it goes
    move.l  d0,d1 d1 holds the value now
