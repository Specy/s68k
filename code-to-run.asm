CR equ $0D
LF equ $0A
str_ask: dc.b 'how many numbers do you want to use?', 0
str_ask_numbers: dc.b 'write next number:',0
new_line: dc.b $0D, $0a, 0

input_length equ 10
input_array: dc.l 10, 3, -1, 2, 0, 4, 17, -4, 8, 2

partial_sums: dcb.l input_length, 0
fenwick_tree: dcb.l input_length, 0

	org $2000
START:

    move.l #128, -(sp)
    bsr p
    move.l (sp)+, d1

    move.l input_array, partial_sums ; ps[0] = a[0]
    move.l #partial_sums, a0
    move.l #input_array, a1
    move.l #4, d0 ;second element
for_start:
    move.l (a1, d0), d1 ; a[i]
    move.l -4(a0, d0), d2 ; ps[i-1]
    add.l d1, d2
    move.l d2, (a0, d0) ; ps[i] = a[i] + ps[i-1]
    add.l #4, d0
    cmp #input_length*4,d0
    blt for_start

    move.l #0, d0
    move.l #input_length, d7
    move.l #$abdc, d1
    move.l #partial_sums, a0
    move.l #fenwick_tree, a2
    *for(int i=0;i<n;i++)
for_start_build_tree:
    move.l d0, -(sp)
    bsr p
    move.l (sp)+, d6 * p(i)
    add.l #1, d0 
    move.l d0, d1
    muls #4, d1
    tst d6
    blt if_less_than_zero 
        *if(p(i) <= 0)
        move.l (a0, d1), d3 * partial_sums[i]
        sub.l #1, d6
        sub.l (a0, d6), d3 * partial_sums[p(i)-1]
        move.l d3, (a2, d1) * fenwick_tree[i] = partial_sums[i] - partial_sums[p(i)-1];
        bra if_end
if_less_than_zero: *else
        move.l (a0, d1), d3 *partial_sums[i];
        move.l d3, (a2, d1) *fenwick_tree[i]
if_end:
    cmp.l #input_length, d0
    blt for_start_build_tree
    bra END


*    int p(int i){
*       return (i & (i+1));
*    }
p_in equ 12
p: 
    move.l d1, -(sp)
    move.l d2, -(sp)
    move.l p_in(sp), d1
    move.l d1, d2
    add.l #1, d2
    and.l d1, d2
    move.l d2, p_in(sp)
    move.l (sp)+, d2
    move.l (sp)+, d1
    rts

print_str: ;reads from a1
    move.l d0, -(sp)
    move.l #14, d0
    trap #15
    move.l (sp)+, d0
    rts

print_new_line:
    move.l d0, -(sp)
    move.l a1, -(sp)
    lea new_line, a1
    move.l #14, d0
    trap #15
    move.l (sp)+, a1
    move.l (sp)+, d0
    rts


ask_number: ;sets to d1
    move.l d0, -(sp)
    move.l #4, d0
    trap #15
    move.l (sp)+, d0
    rts

print_number: ;print number in d1 
    move.l d0, -(sp)
    move.l #3, d0
    trap #15
    move.l (sp)+, d0
    rts

END:




*START:
*    lea str_ask, a1
*    bsr print_str
*    bsr ask_number
*    move.l d1, amount
*    move.l d1, d2
*    bsr print_number
*    bsr print_new_line
*    move.l sp, input_array
*    lea str_ask_numbers, a1
*    sub.l #1, d2
*
*ask_elements: 
*    bsr print_str
*    bsr ask_number
*    move.l d1, -(sp)
*    bsr print_number
*    bsr print_new_line
*    dbra d2, ask_elements
*	bra END