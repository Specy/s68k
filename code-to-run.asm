*-----------------------------------------------------------
* Title      :
* Written by :
* Date       :
* Description:
*-----------------------------------------------------------
    ORG    $1000
    length: dc.w 20
    arr: dc.w 11, 71, 26, 44, 45, 65, 86, 10, 36, 26, 87, 86, 99, 48, 70, 89, 68, 92, 47, 80
START:        
    * sort the array
    move.l #arr, -(sp)
    move.w length,-(sp)
    bsr sort_array
    add.l #6, sp
    * print the sorted array
    move.l #arr, -(sp)
    move.w length,-(sp)
    bsr print_array
    bra end
    
    
    

sa_off_length equ 44
sa_off_array_pointer equ 46
    
sort_array:
    move.w d0, -(sp)
    move.l d1, -(sp)
    move.l d2, -(sp)
    move.l d3, -(sp)
    move.w d4, -(sp)
    move.l d5, -(sp)
    move.l a0, -(sp)
    move.l a1, -(sp)
    move.l a2, -(sp)
    
    move.l d7, -(sp)
    move.l a6, -(sp)
    
    move.w sa_off_length(sp), d7
    muls #2, d7
    move.l sa_off_array_pointer(sp), a6
    * uses:
* d0 (w) = i
* d1 (l) = end
* d2 (l) = j
* d3 (l) = diff
* d4 (w) = tmp
* d5 (l) = swaps
* a0 (l) = toSort
* a1 (l) = beforeElement
* a2 (l) = currentElement

* d7 (w) = parameter length
* a6 (l) = parameter array pointer
* total offset = 34 
    move.w #0, d0 *i = 0
    move.w #0, d5 *swaps = 0
  
for_i_start:
    cmp.w d7, d0
    bge for_i_end *if(i >= length) goto for_i_end
    move.w #2, d2 * j = 1
    move.w d7, d1 * end = length
    sub.w d0, d1  * end -= i
    for_j_start:
        cmp d1, d2
        bge for_j_end   * if(j >= end) goto_for_j_end
        move.l a6, a1   * beforeElement = array pointer
        add.l d2, a1    * beforeElement += j
        sub.l #2, a1    * beforeElement -= 1
        move.l a6, a2   * currentElement = array pointer
        add.l d2, a2    * currentElement += j
        move.w (a1), d3 * diff = *beforeElement
        sub.w (a2), d3  * diff -= *currentElement
        tst d3
        blt if_smaller  * if(diff < 0)
            move.w (a1), d4 * tmp = *beforeElement
            add.l #1, d5 * swaps++
            move.w (a2), (a1) * *beforeElement = *currentElement
            move.w d4, (a2) * *currentElement = tmp
        if_smaller: 
        add.l #2, d2 * j++
        bra for_j_start
    for_j_end:
    add.l #2, d0 * i++
    bra for_i_start
for_i_end:  

    move.l (sp)+, a6
    move.l (sp)+, d7
    move.l (sp)+, a2
    move.l (sp)+, a1
    move.l (sp)+, a0
    move.l (sp)+, d5
    move.w (sp)+, d4
    move.l (sp)+, d3
    move.l (sp)+, d2
    move.l (sp)+, d1
    move.w (sp)+, d0 
    rts
    

* uses:  d0(l) d1(w) d7(w) a2(l) register offset = 12
* total offset = register offset + return = 16
pa_off_length equ 16
pa_off_array_pointer equ 18

print_array:
    move.l d0, -(sp)
    move.w d1, -(sp)
    move.w d7, -(sp)
    move.l a2, -(sp)
    
    move.w pa_off_length(sp), d7
    move.l pa_off_array_pointer(sp), a2
for_start:
    move.l #3, d0
    move.w (a2), d1
    add.l #2, a2
    trap #15
    move.l #6, d0
    move.l #'|', d1
    trap #15
    tst d7
    sub.l #1, d7
    bgt for_start
for_end:
    move.l (sp)+,a2
    move.w (sp)+,d7
    move.w (sp)+,d1
    move.l (sp)+,d0
    rts
    
    
end:
    