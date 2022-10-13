    ORG $1000

START:
    
    move.l #$FF, d0
    swap d0
    move.w #$FFFF, d0
    swap d0
    ext.l d0
