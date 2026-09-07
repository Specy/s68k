count equ 8

    move.w #count-2, d0     ; the outer loop runs count-1 times
outer:
    lea numbers, a0         ; back to the first element
    move.w d0, d1           ; the inner loop is one shorter every pass
inner:
    move.w (a0), d2         ; left = numbers[i]
    move.w 2(a0), d3        ; right = numbers[i + 1]
    cmp.w d2, d3            ; right - left
    bge in_order            ; if(right >= left) leave them alone
    move.w d3, (a0)         ; otherwise swap them
    move.w d2, 2(a0)
in_order:
    addq.l #2, a0           ; on to the next pair
    dbra d1, inner
    dbra d0, outer

    org $2000
numbers: dc.w 42, 8, 15, 4, 23, 16, 99, 1
