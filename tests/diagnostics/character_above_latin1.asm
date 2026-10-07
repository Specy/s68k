* A source character Windows-1252 has no byte for cannot be stored (ADR 0004);
* `€` and the typographic quotes have bytes of their own, and can.
    move.b #'→',d0
    move.b #'€',d1
