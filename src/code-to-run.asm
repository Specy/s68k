START: 
limit equ 10000000
    move.l #limit, d0
loop_start:
	add.l #1, d1
    sub.l #1, d0
    bne loop_start
