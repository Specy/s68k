* `END START` against a label written `start` is what EASy68K's own examples
* do, so the entry point is taken from the label and this says the case differs.
start:
    move.l #1,d0
    rts
    end START
