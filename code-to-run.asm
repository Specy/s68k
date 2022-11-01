a: dc.l 'a','bb'
b: dc.l 'ccc','dddd'

char equ 4

move.l a, d0
move.l a+char, d1
move.l b, d2
move.l b+char, d3
move.l #'ee', d4