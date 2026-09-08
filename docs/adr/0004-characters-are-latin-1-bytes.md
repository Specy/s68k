---
status: accepted
date: 2026-09-07
---

# A character is one Latin-1 byte

Strings were stored as UTF-8, a string read back from memory failed at run time if its bytes were not valid UTF-8, and the display-char and read-char tasks treated a byte as a Latin-1 code, so the same program saw three encodings. We decided that a character is one byte in Latin-1 everywhere: a source character with a code up to 255 becomes that byte and anything beyond is an assembly error naming the character; bytes shown by the terminal decode as Latin-1, which is total, so the runtime error goes away; typed characters are stored the same way. This is the model students are taught (a character is a byte, `'A'` is 65, five letters are five bytes) and it is what EASy68K does, apart from the Windows-1252 codes 128 to 159 that nobody types.

## Considered options

- **UTF-8 everywhere, made consistent**: accented letters would occupy two bytes, `move.b (a0)+,d0` would need two steps per such letter, the display-char task could not show them, and invalid sequences would stay a runtime failure.
- **ASCII only**: simplest, but Italian and Spanish students would lose accented letters in their strings for no gain.

## Consequences

- The asm-editor's terminal decodes program bytes as Latin-1 and refuses or replaces a typed character above 255.
- The golden fixtures of the old assembler differ for any `dc.b` string with a non-ASCII character; those are updated on purpose.
