* The modes phase 3 adds, named rather than called invalid.
    move.w sr,d0
    move.l greeting(pc),d0
greeting: dc.b 'hello',0
