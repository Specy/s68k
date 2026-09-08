* A name that is defined nowhere, offered the closest one that is.
count equ 12
    move.l #cont,d0
* A register that does not exist is a legal name, so it is a symbol; the hint
* says how many registers there are rather than how to define `d8`.
    move.l d8,d1
