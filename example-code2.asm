ORG $1000
length: dc.w 20
arr: dc.w 11, 71, 26, 44, 45
something equ 4
    ; sort the array
move.l #arr, (a0) ;address of array
move.a length,-(sp)
move.w something(d0, a0) , -(sp)
tst d0
beq end

end: