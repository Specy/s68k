move.L #$1, D0   ; Load some values into registers
move.L #$2, D1
move.L #$3, A0
movem.l D0-D1/A0, -(SP) ; Save registers to the stack