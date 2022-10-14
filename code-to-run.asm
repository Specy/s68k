* -------------------------------
* 	LPS Example
*
* 	Data in Memory
*
*	Language: MC68K-ASM1
*	Style: plain MC68000
* 	Version: LPS22-L16-V3-m68k
* -------------------------------

* LPS22 L16 MC68000-ASM1 Solution 3

* Trasforma una parte di un programma Plain C in MC68000, immagazzinando dati
; in memoria allocata staticamente



	org	$1000
inizio_codice:

;	a = b + c;
	move.b	b,d0
	add.b	c,d0
	move.b	d0,a

;	y = x * z + 2300;
	move.w	z,d0
	mulu	x,d0
	add.w	#2300,d0
	move.w	d0,y

;	h = k * a + m;
	move.b	a,d0	; copia il byte che rappresenta a in d0
	ext.w	d0				; converte il byte d0 in word con sign extension
	muls	k,d0	; d0 contiene k * a
	add.w	m,d0
	move.w	d0,h

;	v = w - 160900 - h;
    move.w  d0,h
    move.l  w,v
    sub.l   #160900,v
    ext.l   d0
    sub.l   d0,v

;	h = 780;
    move.w  #780,h
    
;	k = -24078 % m + 4 * h;
    move.l  #-24078,d0
    divs    m,d0
    swap    d0
    move.w  h,d1
    asl.w   #2,d1
    add.w   d1,d0
    move.w  d0,k


fine_codice:

* sezione dati
	org	$00002800

* esempio di direttiva di definizione dati
; dc.l	1,3,4	; alloca 3 long, inizializzate ai valori 1, 3 e 4

* Stabiliamo la corrispondenza tra variabili della versione Ref-pc e parole di
* memoria m68k, mediante label

* a ciascuna variabile long corrisponde una LONG (32 bit) di memoria
; long v, w = 220200;
v:	dc.l	0
w:	dc.l	220200	

* a ciascuna variabile short (signed o unsigned) corrisponde una WORD (16 bit) di memoria
; unsigned short x = 450, y, z = 98;
; signed short h, k = 600, m = 12;
x:	dc.w	450
y:	dc.w	-1
z:	dc.w	98
h:	dc.w	-1
k:	dc.w	600
m:	dc.w	12
	

* a ciascuna variabile signed char corrisponde un BYTE (8 bit) di memoria
; signed char a, b = 'A', c = -43;
a:	dc.b	-1
b:	dc.b	'A'
c:	dc.b	-43
	
	end:
	