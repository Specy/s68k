* A character literal holds four characters, which is a long; beyond that
* EASy68K warns ("ASCII constant exceeds 4 characters") and so does this.
* The message names the literal, and not the number it packs to.
    move.l #'abcdefgh',d0
    move.w #'toolong',d1
* Four is fine, and a string of a `dc` is not a character literal at all.
    move.l #'abcd',d2
greeting: dc.b 'hello, world',0
