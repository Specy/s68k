---
status: accepted
date: 2026-09-07
---

# A character is one Latin-1 byte

*Amended on 2026-10-05: the byte is Windows-1252's, as in EASy68K, rather than Latin-1's; see [the amendment](#amendment-2026-10-05-windows-1252-as-easy68k) at the end.*

Strings were stored as UTF-8, a string read back from memory failed at run time if its bytes were not valid UTF-8, and the display-char and read-char tasks treated a byte as a Latin-1 code, so the same program saw three encodings. We decided that a character is one byte in Latin-1 everywhere: a source character with a code up to 255 becomes that byte and anything beyond is an assembly error naming the character; bytes shown by the terminal decode as Latin-1, which is total, so the runtime error goes away; typed characters are stored the same way. This is the model students are taught (a character is a byte, `'A'` is 65, five letters are five bytes) and it is what EASy68K does, apart from the Windows-1252 codes 128 to 159 that nobody types.

## Considered options

- **UTF-8 everywhere, made consistent**: accented letters would occupy two bytes, `move.b (a0)+,d0` would need two steps per such letter, the display-char task could not show them, and invalid sequences would stay a runtime failure.
- **ASCII only**: simplest, but Italian and Spanish students would lose accented letters in their strings for no gain.

## Consequences

- The asm-editor's terminal decodes program bytes as Latin-1 and refuses or replaces a typed character above 255.
- The golden fixtures of the old assembler differ for any `dc.b` string with a non-ASCII character; those are updated on purpose.

## Amendment, 2026-10-05: Windows-1252, as EASy68K

The character set is **Windows-1252**, not Latin-1. EASy68K is a Windows program in the code page Windows-1252, so a source file it wrote, a string it displays and a key typed into it are Windows-1252 bytes; Windows-1252 is Latin-1 with 27 printable characters — `€ ‚ ƒ „ … † ‡ ˆ ‰ Š ‹ Œ Ž ‘ ’ “ ” • – — ˜ ™ š › œ ž Ÿ` — where Latin-1 has the control codes `$80` to `$9F`. With Latin-1, an EASy68K source using any of them, typically a `€` or a typographic quote typed in its editor, failed to assemble, and the same byte displayed as an invisible control code; the codes this decision called "nobody types" are exactly what a word processor and a European keyboard produce. The asm-editor's design for its environments holds every Target to its reference environment, which for M68K is EASy68K, and asked for this change.

- A source character is stored as its Windows-1252 byte. A character Windows-1252 has no byte for — one above `ÿ` that is not among the 27, or one of the control codes `$80` to `$9F` they replaced — is the assembly error it was, still under the code `character_above_latin1`, whose name is older than this amendment and stays because a code never changes once it has shipped. A typographic quote or dash outside a string now has a byte, so it is `unexpected_character` there, and its hint still names the plain character to write; inside a string it is the byte EASy68K stores.
- The five codes Windows-1252 leaves undefined (`$81`, `$8D`, `$8F`, `$90`, `$9D`) are the control characters of the same number, as the WHATWG Encoding Standard has them and a browser's `TextDecoder("windows-1252")` decodes them, so decoding stays total and every byte encodes back to itself.
- The text tasks of `trap #15` decode the bytes they display with the same table, in the Core, and a line or a key the user types is encoded with it before it reaches memory, a character with no byte becoming `?` as Windows makes it. The command line reads a source file that is not UTF-8 as Windows-1252.
- The file and sound tasks decode the names they take with the same table, and task 58 encodes the path a dialog chose with it, `?` for a character with no byte (2026-10-06).
- The consequence "The asm-editor's terminal decodes program bytes as Latin-1" above no longer holds: the Core hands the host text, decoded, and a host that decodes bytes itself does so as Windows-1252.
