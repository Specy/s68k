* A character that starts no token at all.
    move.l ?,d0
* A typographic quote has a byte, and starts nothing either: the hint names
* the quote that was meant.
    move.b #‘a’,d1
