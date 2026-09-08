    lea text, a0        ; left = text
    move.l a0, a1       ; right = text
find_end:
    tst.b (a1)+         ; walk to the terminator
    bne find_end
    subq.l #2, a1       ; back onto the last character
swap_loop:
    cmp.l a0, a1        ; right - left
    bls done            ; while(left < right)
    move.b (a0), d0     ; t = *left
    move.b (a1), d1     ; u = *right
    move.b d1, (a0)+    ; *left++ = u
    move.b d0, (a1)     ; *right = t
    subq.l #1, a1       ; right--
    bra swap_loop
done:

    org $2000
text: dc.b 'Assembly', 0
