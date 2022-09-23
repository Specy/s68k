ORG    $1000
length: dc.w 20
arr: dc.w 11, 71, 26, 44, 45
move.b #255, d0
move.l #arr, (a0)
move.w length,-(sp)