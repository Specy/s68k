move.w #100, d1
muls #4, d1
sub #400, d1
tst d1
beq test
move.l #$dead, d5
bra end
test:
move #4, d0
trap #15
move.l #$beef, d4
end: